import{a as Sn,b as ze,c as Ke,d as h,e as l,f as Oh,g as Lh,h as rl,i as R,j as sl}from"./chunks/chunk-U5JLL4QM.js";var ev=Sn(pl=>{"use strict";var nR=Symbol.for("react.transitional.element"),rR=Symbol.for("react.fragment");function Wh(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:nR,type:e,key:n,ref:t!==void 0?t:null,props:a}}pl.Fragment=rR;pl.jsx=Wh;pl.jsxs=Wh});var yd=Sn((cL,tv)=>{"use strict";tv.exports=ev()});var hv=Sn(Me=>{"use strict";function _d(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Sl(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Da(e){return e.length===0?null:e[0]}function _l(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>Sl(o,a))u<r&&0>Sl(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>Sl(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function Sl(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Me.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(iv=performance,Me.unstable_now=function(){return iv.now()}):(wd=Date,ov=wd.now(),Me.unstable_now=function(){return wd.now()-ov});var iv,wd,ov,Xa=[],kn=[],lR=1,la=null,yt=3,kd=!1,_i=!1,ki=!1,Rd=!1,cv=typeof setTimeout=="function"?setTimeout:null,dv=typeof clearTimeout=="function"?clearTimeout:null,lv=typeof setImmediate<"u"?setImmediate:null;function Nl(e){for(var t=Da(kn);t!==null;){if(t.callback===null)_l(kn);else if(t.startTime<=e)_l(kn),t.sortIndex=t.expirationTime,_d(Xa,t);else break;t=Da(kn)}}function Cd(e){if(ki=!1,Nl(e),!_i)if(Da(Xa)!==null)_i=!0,Jr||(Jr=!0,Yr());else{var t=Da(kn);t!==null&&Ed(Cd,t.startTime-e)}}var Jr=!1,Ri=-1,mv=5,fv=-1;function pv(){return Rd?!0:!(Me.unstable_now()-fv<mv)}function Sd(){if(Rd=!1,Jr){var e=Me.unstable_now();fv=e;var t=!0;try{e:{_i=!1,ki&&(ki=!1,dv(Ri),Ri=-1),kd=!0;var a=yt;try{t:{for(Nl(e),la=Da(Xa);la!==null&&!(la.expirationTime>e&&pv());){var n=la.callback;if(typeof n=="function"){la.callback=null,yt=la.priorityLevel;var r=n(la.expirationTime<=e);if(e=Me.unstable_now(),typeof r=="function"){la.callback=r,Nl(e),t=!0;break t}la===Da(Xa)&&_l(Xa),Nl(e)}else _l(Xa);la=Da(Xa)}if(la!==null)t=!0;else{var s=Da(kn);s!==null&&Ed(Cd,s.startTime-e),t=!1}}break e}finally{la=null,yt=a,kd=!1}t=void 0}}finally{t?Yr():Jr=!1}}}var Yr;typeof lv=="function"?Yr=function(){lv(Sd)}:typeof MessageChannel<"u"?(Nd=new MessageChannel,uv=Nd.port2,Nd.port1.onmessage=Sd,Yr=function(){uv.postMessage(null)}):Yr=function(){cv(Sd,0)};var Nd,uv;function Ed(e,t){Ri=cv(function(){e(Me.unstable_now())},t)}Me.unstable_IdlePriority=5;Me.unstable_ImmediatePriority=1;Me.unstable_LowPriority=4;Me.unstable_NormalPriority=3;Me.unstable_Profiling=null;Me.unstable_UserBlockingPriority=2;Me.unstable_cancelCallback=function(e){e.callback=null};Me.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):mv=0<e?Math.floor(1e3/e):5};Me.unstable_getCurrentPriorityLevel=function(){return yt};Me.unstable_next=function(e){switch(yt){case 1:case 2:case 3:var t=3;break;default:t=yt}var a=yt;yt=t;try{return e()}finally{yt=a}};Me.unstable_requestPaint=function(){Rd=!0};Me.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=yt;yt=e;try{return t()}finally{yt=a}};Me.unstable_scheduleCallback=function(e,t,a){var n=Me.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:lR++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,_d(kn,e),Da(Xa)===null&&e===Da(kn)&&(ki?(dv(Ri),Ri=-1):ki=!0,Ed(Cd,a-n))):(e.sortIndex=r,_d(Xa,e),_i||kd||(_i=!0,Jr||(Jr=!0,Yr()))),e};Me.unstable_shouldYield=pv;Me.unstable_wrapCallback=function(e){var t=yt;return function(){var a=yt;yt=t;try{return e.apply(this,arguments)}finally{yt=a}}}});var gv=Sn((QL,vv)=>{"use strict";vv.exports=hv()});var bv=Sn(kt=>{"use strict";var uR=Ke();function yv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Rn(){}var _t={d:{f:Rn,r:function(){throw Error(yv(522))},D:Rn,C:Rn,L:Rn,m:Rn,X:Rn,S:Rn,M:Rn},p:0,findDOMNode:null},cR=Symbol.for("react.portal");function dR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:cR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Ci=uR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function kl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}kt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=_t;kt.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(yv(299));return dR(e,t,null,a)};kt.flushSync=function(e){var t=Ci.T,a=_t.p;try{if(Ci.T=null,_t.p=2,e)return e()}finally{Ci.T=t,_t.p=a,_t.d.f()}};kt.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,_t.d.C(e,t))};kt.prefetchDNS=function(e){typeof e=="string"&&_t.d.D(e)};kt.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=kl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?_t.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&_t.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};kt.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=kl(t.as,t.crossOrigin);_t.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&_t.d.M(e)};kt.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=kl(a,t.crossOrigin);_t.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};kt.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=kl(t.as,t.crossOrigin);_t.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else _t.d.m(e)};kt.requestFormReset=function(e){_t.d.r(e)};kt.unstable_batchedUpdates=function(e,t){return e(t)};kt.useFormState=function(e,t,a){return Ci.H.useFormState(e,t,a)};kt.useFormStatus=function(){return Ci.H.useHostTransitionStatus()};kt.version="19.1.0"});var wv=Sn((GL,$v)=>{"use strict";function xv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(xv)}catch(e){console.error(e)}}xv(),$v.exports=bv()});var N0=Sn(Gu=>{"use strict";var rt=gv(),Kg=Ke(),mR=wv();function P(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Hg(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function vo(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function Qg(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Sv(e){if(vo(e)!==e)throw Error(P(188))}function fR(e){var t=e.alternate;if(!t){if(t=vo(e),t===null)throw Error(P(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Sv(r),e;if(s===n)return Sv(r),t;s=s.sibling}throw Error(P(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(P(189))}}if(a.alternate!==n)throw Error(P(190))}if(a.tag!==3)throw Error(P(188));return a.stateNode.current===a?e:t}function Vg(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=Vg(e),t!==null)return t;e=e.sibling}return null}var Te=Object.assign,pR=Symbol.for("react.element"),Rl=Symbol.for("react.transitional.element"),Ui=Symbol.for("react.portal"),ns=Symbol.for("react.fragment"),Gg=Symbol.for("react.strict_mode"),om=Symbol.for("react.profiler"),hR=Symbol.for("react.provider"),Yg=Symbol.for("react.consumer"),an=Symbol.for("react.context"),af=Symbol.for("react.forward_ref"),lm=Symbol.for("react.suspense"),um=Symbol.for("react.suspense_list"),nf=Symbol.for("react.memo"),Tn=Symbol.for("react.lazy");Symbol.for("react.scope");var cm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var vR=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Nv=Symbol.iterator;function Ei(e){return e===null||typeof e!="object"?null:(e=Nv&&e[Nv]||e["@@iterator"],typeof e=="function"?e:null)}var gR=Symbol.for("react.client.reference");function dm(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===gR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case ns:return"Fragment";case om:return"Profiler";case Gg:return"StrictMode";case lm:return"Suspense";case um:return"SuspenseList";case cm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Ui:return"Portal";case an:return(e.displayName||"Context")+".Provider";case Yg:return(e._context.displayName||"Context")+".Consumer";case af:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case nf:return t=e.displayName||null,t!==null?t:dm(e.type)||"Memo";case Tn:t=e._payload,e=e._init;try{return dm(e(t))}catch{}}return null}var ji=Array.isArray,ae=Kg.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,pe=mR.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,fr={pending:!1,data:null,method:null,action:null},mm=[],rs=-1;function Fa(e){return{current:e}}function dt(e){0>rs||(e.current=mm[rs],mm[rs]=null,rs--)}function Le(e,t){rs++,mm[rs]=e.current,e.current=t}var Pa=Fa(null),to=Fa(null),qn=Fa(null),nu=Fa(null);function ru(e,t){switch(Le(qn,t),Le(to,e),Le(Pa,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?Tg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=Tg(t),e=m0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}dt(Pa),Le(Pa,e)}function Ss(){dt(Pa),dt(to),dt(qn)}function fm(e){e.memoizedState!==null&&Le(nu,e);var t=Pa.current,a=m0(t,e.type);t!==a&&(Le(to,e),Le(Pa,a))}function su(e){to.current===e&&(dt(Pa),dt(to)),nu.current===e&&(dt(nu),mo._currentValue=fr)}var pm=Object.prototype.hasOwnProperty,rf=rt.unstable_scheduleCallback,Td=rt.unstable_cancelCallback,yR=rt.unstable_shouldYield,bR=rt.unstable_requestPaint,Ua=rt.unstable_now,xR=rt.unstable_getCurrentPriorityLevel,Jg=rt.unstable_ImmediatePriority,Xg=rt.unstable_UserBlockingPriority,iu=rt.unstable_NormalPriority,$R=rt.unstable_LowPriority,Zg=rt.unstable_IdlePriority,wR=rt.log,SR=rt.unstable_setDisableYieldValue,go=null,Yt=null;function Pn(e){if(typeof wR=="function"&&SR(e),Yt&&typeof Yt.setStrictMode=="function")try{Yt.setStrictMode(go,e)}catch{}}var Jt=Math.clz32?Math.clz32:kR,NR=Math.log,_R=Math.LN2;function kR(e){return e>>>=0,e===0?32:31-(NR(e)/_R|0)|0}var Cl=256,El=4194304;function cr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Mu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=cr(n):(i&=o,i!==0?r=cr(i):a||(a=o&~e,a!==0&&(r=cr(a))))):(o=n&~s,o!==0?r=cr(o):i!==0?r=cr(i):a||(a=n&~e,a!==0&&(r=cr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function yo(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function RR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function Wg(){var e=Cl;return Cl<<=1,(Cl&4194048)===0&&(Cl=256),e}function ey(){var e=El;return El<<=1,(El&62914560)===0&&(El=4194304),e}function Ad(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function bo(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function CR(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Jt(a),f=1<<d;o[d]=0,u[d]=-1;var m=c[d];if(m!==null)for(c[d]=null,d=0;d<m.length;d++){var p=m[d];p!==null&&(p.lane&=-536870913)}a&=~f}n!==0&&ty(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function ty(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Jt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function ay(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Jt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function sf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function of(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function ny(){var e=pe.p;return e!==0?e:(e=window.event,e===void 0?32:w0(e.type))}function ER(e,t){var a=pe.p;try{return pe.p=e,t()}finally{pe.p=a}}var Xn=Math.random().toString(36).slice(2),bt="__reactFiber$"+Xn,Ft="__reactProps$"+Xn,Os="__reactContainer$"+Xn,hm="__reactEvents$"+Xn,TR="__reactListeners$"+Xn,AR="__reactHandles$"+Xn,_v="__reactResources$"+Xn,xo="__reactMarker$"+Xn;function lf(e){delete e[bt],delete e[Ft],delete e[hm],delete e[TR],delete e[AR]}function ss(e){var t=e[bt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Os]||a[bt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Mg(e);e!==null;){if(a=e[bt])return a;e=Mg(e)}return t}e=a,a=e.parentNode}return null}function Ls(e){if(e=e[bt]||e[Os]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Fi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(P(33))}function hs(e){var t=e[_v];return t||(t=e[_v]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ut(e){e[xo]=!0}var ry=new Set,sy={};function Nr(e,t){Ns(e,t),Ns(e+"Capture",t)}function Ns(e,t){for(sy[e]=t,e=0;e<t.length;e++)ry.add(t[e])}var DR=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),kv={},Rv={};function MR(e){return pm.call(Rv,e)?!0:pm.call(kv,e)?!1:DR.test(e)?Rv[e]=!0:(kv[e]=!0,!1)}function Kl(e,t,a){if(MR(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Tl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function Za(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var Dd,Cv;function es(e){if(Dd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);Dd=t&&t[1]||"",Cv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+Dd+e+Cv}var Md=!1;function Od(e,t){if(!e||Md)return"";Md=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var f=function(){throw Error()};if(Object.defineProperty(f.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(f,[])}catch(p){var m=p}Reflect.construct(e,[],f)}else{try{f.call()}catch(p){m=p}e.call(f.prototype)}}else{try{throw Error()}catch(p){m=p}(f=e())&&typeof f.catch=="function"&&f.catch(function(){})}}catch(p){if(p&&m&&typeof p.stack=="string")return[p.stack,m.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Md=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?es(a):""}function OR(e){switch(e.tag){case 26:case 27:case 5:return es(e.type);case 16:return es("Lazy");case 13:return es("Suspense");case 19:return es("SuspenseList");case 0:case 15:return Od(e.type,!1);case 11:return Od(e.type.render,!1);case 1:return Od(e.type,!0);case 31:return es("Activity");default:return""}}function Ev(e){try{var t="";do t+=OR(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function ca(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function iy(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function LR(e){var t=iy(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function ou(e){e._valueTracker||(e._valueTracker=LR(e))}function oy(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=iy(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function lu(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var PR=/[\n"\\]/g;function fa(e){return e.replace(PR,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function vm(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+ca(t)):e.value!==""+ca(t)&&(e.value=""+ca(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?gm(e,i,ca(t)):a!=null?gm(e,i,ca(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+ca(o):e.removeAttribute("name")}function ly(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+ca(a):"",t=t!=null?""+ca(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function gm(e,t,a){t==="number"&&lu(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function vs(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+ca(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function uy(e,t,a){if(t!=null&&(t=""+ca(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+ca(a):""}function cy(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(P(92));if(ji(n)){if(1<n.length)throw Error(P(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=ca(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function _s(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var UR=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function Tv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||UR.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function dy(e,t,a){if(t!=null&&typeof t!="object")throw Error(P(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&Tv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&Tv(e,s,t[s])}function uf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var jR=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),FR=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function Hl(e){return FR.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var ym=null;function cf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var is=null,gs=null;function Av(e){var t=Ls(e);if(t&&(e=t.stateNode)){var a=e[Ft]||null;e:switch(e=t.stateNode,t.type){case"input":if(vm(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+fa(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[Ft]||null;if(!r)throw Error(P(90));vm(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&oy(n)}break e;case"textarea":uy(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&vs(e,!!a.multiple,t,!1)}}}var Ld=!1;function my(e,t,a){if(Ld)return e(t,a);Ld=!0;try{var n=e(t);return n}finally{if(Ld=!1,(is!==null||gs!==null)&&(Iu(),is&&(t=is,e=gs,gs=is=null,Av(t),e)))for(t=0;t<e.length;t++)Av(e[t])}}function ao(e,t){var a=e.stateNode;if(a===null)return null;var n=a[Ft]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(P(231,t,typeof a));return a}var cn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),bm=!1;if(cn)try{Xr={},Object.defineProperty(Xr,"passive",{get:function(){bm=!0}}),window.addEventListener("test",Xr,Xr),window.removeEventListener("test",Xr,Xr)}catch{bm=!1}var Xr,Un=null,df=null,Ql=null;function fy(){if(Ql)return Ql;var e,t=df,a=t.length,n,r="value"in Un?Un.value:Un.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return Ql=r.slice(e,1<n?1-n:void 0)}function Vl(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function Al(){return!0}function Dv(){return!1}function qt(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?Al:Dv,this.isPropagationStopped=Dv,this}return Te(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=Al)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=Al)},persist:function(){},isPersistent:Al}),t}var _r={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Ou=qt(_r),$o=Te({},_r,{view:0,detail:0}),qR=qt($o),Pd,Ud,Ti,Lu=Te({},$o,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:mf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==Ti&&(Ti&&e.type==="mousemove"?(Pd=e.screenX-Ti.screenX,Ud=e.screenY-Ti.screenY):Ud=Pd=0,Ti=e),Pd)},movementY:function(e){return"movementY"in e?e.movementY:Ud}}),Mv=qt(Lu),zR=Te({},Lu,{dataTransfer:0}),BR=qt(zR),IR=Te({},$o,{relatedTarget:0}),jd=qt(IR),KR=Te({},_r,{animationName:0,elapsedTime:0,pseudoElement:0}),HR=qt(KR),QR=Te({},_r,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),VR=qt(QR),GR=Te({},_r,{data:0}),Ov=qt(GR),YR={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},JR={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},XR={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function ZR(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=XR[e])?!!t[e]:!1}function mf(){return ZR}var WR=Te({},$o,{key:function(e){if(e.key){var t=YR[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=Vl(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?JR[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:mf,charCode:function(e){return e.type==="keypress"?Vl(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?Vl(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),eC=qt(WR),tC=Te({},Lu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Lv=qt(tC),aC=Te({},$o,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:mf}),nC=qt(aC),rC=Te({},_r,{propertyName:0,elapsedTime:0,pseudoElement:0}),sC=qt(rC),iC=Te({},Lu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),oC=qt(iC),lC=Te({},_r,{newState:0,oldState:0}),uC=qt(lC),cC=[9,13,27,32],ff=cn&&"CompositionEvent"in window,zi=null;cn&&"documentMode"in document&&(zi=document.documentMode);var dC=cn&&"TextEvent"in window&&!zi,py=cn&&(!ff||zi&&8<zi&&11>=zi),Pv=" ",Uv=!1;function hy(e,t){switch(e){case"keyup":return cC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function vy(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var os=!1;function mC(e,t){switch(e){case"compositionend":return vy(t);case"keypress":return t.which!==32?null:(Uv=!0,Pv);case"textInput":return e=t.data,e===Pv&&Uv?null:e;default:return null}}function fC(e,t){if(os)return e==="compositionend"||!ff&&hy(e,t)?(e=fy(),Ql=df=Un=null,os=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return py&&t.locale!=="ko"?null:t.data;default:return null}}var pC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function jv(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!pC[e.type]:t==="textarea"}function gy(e,t,a,n){is?gs?gs.push(n):gs=[n]:is=n,t=ku(t,"onChange"),0<t.length&&(a=new Ou("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var Bi=null,no=null;function hC(e){u0(e,0)}function Pu(e){var t=Fi(e);if(oy(t))return e}function Fv(e,t){if(e==="change")return t}var yy=!1;cn&&(cn?(Ml="oninput"in document,Ml||(Fd=document.createElement("div"),Fd.setAttribute("oninput","return;"),Ml=typeof Fd.oninput=="function"),Dl=Ml):Dl=!1,yy=Dl&&(!document.documentMode||9<document.documentMode));var Dl,Ml,Fd;function qv(){Bi&&(Bi.detachEvent("onpropertychange",by),no=Bi=null)}function by(e){if(e.propertyName==="value"&&Pu(no)){var t=[];gy(t,no,e,cf(e)),my(hC,t)}}function vC(e,t,a){e==="focusin"?(qv(),Bi=t,no=a,Bi.attachEvent("onpropertychange",by)):e==="focusout"&&qv()}function gC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Pu(no)}function yC(e,t){if(e==="click")return Pu(t)}function bC(e,t){if(e==="input"||e==="change")return Pu(t)}function xC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var Wt=typeof Object.is=="function"?Object.is:xC;function ro(e,t){if(Wt(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!pm.call(t,r)||!Wt(e[r],t[r]))return!1}return!0}function zv(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function Bv(e,t){var a=zv(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=zv(a)}}function xy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?xy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function $y(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=lu(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=lu(e.document)}return t}function pf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var $C=cn&&"documentMode"in document&&11>=document.documentMode,ls=null,xm=null,Ii=null,$m=!1;function Iv(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;$m||ls==null||ls!==lu(n)||(n=ls,"selectionStart"in n&&pf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),Ii&&ro(Ii,n)||(Ii=n,n=ku(xm,"onSelect"),0<n.length&&(t=new Ou("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=ls)))}function ur(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var us={animationend:ur("Animation","AnimationEnd"),animationiteration:ur("Animation","AnimationIteration"),animationstart:ur("Animation","AnimationStart"),transitionrun:ur("Transition","TransitionRun"),transitionstart:ur("Transition","TransitionStart"),transitioncancel:ur("Transition","TransitionCancel"),transitionend:ur("Transition","TransitionEnd")},qd={},wy={};cn&&(wy=document.createElement("div").style,"AnimationEvent"in window||(delete us.animationend.animation,delete us.animationiteration.animation,delete us.animationstart.animation),"TransitionEvent"in window||delete us.transitionend.transition);function kr(e){if(qd[e])return qd[e];if(!us[e])return e;var t=us[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in wy)return qd[e]=t[a];return e}var Sy=kr("animationend"),Ny=kr("animationiteration"),_y=kr("animationstart"),wC=kr("transitionrun"),SC=kr("transitionstart"),NC=kr("transitioncancel"),ky=kr("transitionend"),Ry=new Map,wm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");wm.push("scrollEnd");function Sa(e,t){Ry.set(e,t),Nr(t,[e])}var Kv=new WeakMap;function pa(e,t){if(typeof e=="object"&&e!==null){var a=Kv.get(e);return a!==void 0?a:(t={value:e,source:t,stack:Ev(t)},Kv.set(e,t),t)}return{value:e,source:t,stack:Ev(t)}}var ua=[],cs=0,hf=0;function Uu(){for(var e=cs,t=hf=cs=0;t<e;){var a=ua[t];ua[t++]=null;var n=ua[t];ua[t++]=null;var r=ua[t];ua[t++]=null;var s=ua[t];if(ua[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&Cy(a,r,s)}}function ju(e,t,a,n){ua[cs++]=e,ua[cs++]=t,ua[cs++]=a,ua[cs++]=n,hf|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function vf(e,t,a,n){return ju(e,t,a,n),uu(e)}function Ps(e,t){return ju(e,null,null,t),uu(e)}function Cy(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Jt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function uu(e){if(50<Wi)throw Wi=0,Im=null,Error(P(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var ds={};function _C(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Gt(e,t,a,n){return new _C(e,t,a,n)}function gf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function ln(e,t){var a=e.alternate;return a===null?(a=Gt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function Ey(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function Gl(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")gf(e)&&(i=1);else if(typeof e=="string")i=_3(e,a,Pa.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case cm:return e=Gt(31,a,t,r),e.elementType=cm,e.lanes=s,e;case ns:return pr(a.children,r,s,t);case Gg:i=8,r|=24;break;case om:return e=Gt(12,a,t,r|2),e.elementType=om,e.lanes=s,e;case lm:return e=Gt(13,a,t,r),e.elementType=lm,e.lanes=s,e;case um:return e=Gt(19,a,t,r),e.elementType=um,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case hR:case an:i=10;break e;case Yg:i=9;break e;case af:i=11;break e;case nf:i=14;break e;case Tn:i=16,n=null;break e}i=29,a=Error(P(130,e===null?"null":typeof e,"")),n=null}return t=Gt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function pr(e,t,a,n){return e=Gt(7,e,n,t),e.lanes=a,e}function zd(e,t,a){return e=Gt(6,e,null,t),e.lanes=a,e}function Bd(e,t,a){return t=Gt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var ms=[],fs=0,cu=null,du=0,da=[],ma=0,hr=null,nn=1,rn="";function dr(e,t){ms[fs++]=du,ms[fs++]=cu,cu=e,du=t}function Ty(e,t,a){da[ma++]=nn,da[ma++]=rn,da[ma++]=hr,hr=e;var n=nn;e=rn;var r=32-Jt(n)-1;n&=~(1<<r),a+=1;var s=32-Jt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,nn=1<<32-Jt(t)+r|a<<r|n,rn=s+e}else nn=1<<s|a<<r|n,rn=e}function yf(e){e.return!==null&&(dr(e,1),Ty(e,1,0))}function bf(e){for(;e===cu;)cu=ms[--fs],ms[fs]=null,du=ms[--fs],ms[fs]=null;for(;e===hr;)hr=da[--ma],da[ma]=null,rn=da[--ma],da[ma]=null,nn=da[--ma],da[ma]=null}var Rt=null,Be=null,fe=!1,vr=null,Oa=!1,Sm=Error(P(519));function xr(e){var t=Error(P(418,""));throw so(pa(t,e)),Sm}function Hv(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[bt]=e,t[Ft]=n,a){case"dialog":oe("cancel",t),oe("close",t);break;case"iframe":case"object":case"embed":oe("load",t);break;case"video":case"audio":for(a=0;a<lo.length;a++)oe(lo[a],t);break;case"source":oe("error",t);break;case"img":case"image":case"link":oe("error",t),oe("load",t);break;case"details":oe("toggle",t);break;case"input":oe("invalid",t),ly(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),ou(t);break;case"select":oe("invalid",t);break;case"textarea":oe("invalid",t),cy(t,n.value,n.defaultValue,n.children),ou(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||d0(t.textContent,a)?(n.popover!=null&&(oe("beforetoggle",t),oe("toggle",t)),n.onScroll!=null&&oe("scroll",t),n.onScrollEnd!=null&&oe("scrollend",t),n.onClick!=null&&(t.onclick=Qu),t=!0):t=!1,t||xr(e)}function Qv(e){for(Rt=e.return;Rt;)switch(Rt.tag){case 5:case 13:Oa=!1;return;case 27:case 3:Oa=!0;return;default:Rt=Rt.return}}function Ai(e){if(e!==Rt)return!1;if(!fe)return Qv(e),fe=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||Ym(e.type,e.memoizedProps)),a=!a),a&&Be&&xr(e),Qv(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(P(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){Be=wa(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}Be=null}}else t===27?(t=Be,Zn(e.type)?(e=Zm,Zm=null,Be=e):Be=t):Be=Rt?wa(e.stateNode.nextSibling):null;return!0}function wo(){Be=Rt=null,fe=!1}function Vv(){var e=vr;return e!==null&&(jt===null?jt=e:jt.push.apply(jt,e),vr=null),e}function so(e){vr===null?vr=[e]:vr.push(e)}var Nm=Fa(null),Rr=null,sn=null;function Dn(e,t,a){Le(Nm,t._currentValue),t._currentValue=a}function un(e){e._currentValue=Nm.current,dt(Nm)}function _m(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function km(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),_m(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(P(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),_m(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function So(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(P(387));if(i=i.memoizedProps,i!==null){var o=r.type;Wt(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===nu.current){if(i=r.alternate,i===null)throw Error(P(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(mo):e=[mo])}r=r.return}e!==null&&km(t,e,a,n),t.flags|=262144}function mu(e){for(e=e.firstContext;e!==null;){if(!Wt(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function $r(e){Rr=e,sn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function xt(e){return Ay(Rr,e)}function Ol(e,t){return Rr===null&&$r(e),Ay(e,t)}function Ay(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},sn===null){if(e===null)throw Error(P(308));sn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else sn=sn.next=t;return a}var kC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},RC=rt.unstable_scheduleCallback,CC=rt.unstable_NormalPriority,at={$$typeof:an,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function xf(){return{controller:new kC,data:new Map,refCount:0}}function No(e){e.refCount--,e.refCount===0&&RC(CC,function(){e.controller.abort()})}var Ki=null,Rm=0,ks=0,ys=null;function EC(e,t){if(Ki===null){var a=Ki=[];Rm=0,ks=If(),ys={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Rm++,t.then(Gv,Gv),t}function Gv(){if(--Rm===0&&Ki!==null){ys!==null&&(ys.status="fulfilled");var e=Ki;Ki=null,ks=0,ys=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function TC(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var Yv=ae.S;ae.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&EC(e,t),Yv!==null&&Yv(e,t)};var gr=Fa(null);function $f(){var e=gr.current;return e!==null?e:ke.pooledCache}function Yl(e,t){t===null?Le(gr,gr.current):Le(gr,t.pool)}function Dy(){var e=$f();return e===null?null:{parent:at._currentValue,pool:e}}var _o=Error(P(460)),My=Error(P(474)),Fu=Error(P(542)),Cm={then:function(){}};function Jv(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Ll(){}function Oy(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Ll,Ll),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Zv(e),e;default:if(typeof t.status=="string")t.then(Ll,Ll);else{if(e=ke,e!==null&&100<e.shellSuspendCounter)throw Error(P(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,Zv(e),e}throw Hi=t,_o}}var Hi=null;function Xv(){if(Hi===null)throw Error(P(459));var e=Hi;return Hi=null,e}function Zv(e){if(e===_o||e===Fu)throw Error(P(483))}var An=!1;function wf(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Em(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function zn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Bn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(be&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=uu(e),Cy(e,null,a),t}return ju(e,n,t,a),uu(e)}function Qi(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,ay(e,a)}}function Id(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Tm=!1;function Vi(){if(Tm){var e=ys;if(e!==null)throw e}}function Gi(e,t,a,n){Tm=!1;var r=e.updateQueue;An=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var f=r.baseState;i=0,d=c=u=null,o=s;do{var m=o.lane&-536870913,p=m!==o.lane;if(p?(ce&m)===m:(n&m)===m){m!==0&&m===ks&&(Tm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var b=e,y=o;m=t;var $=a;switch(y.tag){case 1:if(b=y.payload,typeof b=="function"){f=b.call($,f,m);break e}f=b;break e;case 3:b.flags=b.flags&-65537|128;case 0:if(b=y.payload,m=typeof b=="function"?b.call($,f,m):b,m==null)break e;f=Te({},f,m);break e;case 2:An=!0}}m=o.callback,m!==null&&(e.flags|=64,p&&(e.flags|=8192),p=r.callbacks,p===null?r.callbacks=[m]:p.push(m))}else p={lane:m,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=p,u=f):d=d.next=p,i|=m;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;p=o,o=p.next,p.next=null,r.lastBaseUpdate=p,r.shared.pending=null}}while(!0);d===null&&(u=f),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),Jn|=i,e.lanes=i,e.memoizedState=f}}function Ly(e,t){if(typeof e!="function")throw Error(P(191,e));e.call(t)}function Py(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Ly(a[e],t)}var Rs=Fa(null),fu=Fa(0);function Wv(e,t){e=fn,Le(fu,e),Le(Rs,t),fn=e|t.baseLanes}function Am(){Le(fu,fn),Le(Rs,Rs.current)}function Sf(){fn=fu.current,dt(Rs),dt(fu)}var Gn=0,se=null,we=null,Je=null,pu=!1,bs=!1,wr=!1,hu=0,io=0,xs=null,AC=0;function He(){throw Error(P(321))}function Nf(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!Wt(e[a],t[a]))return!1;return!0}function _f(e,t,a,n,r,s){return Gn=s,se=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,ae.H=e===null||e.memoizedState===null?fb:pb,wr=!1,s=a(n,r),wr=!1,bs&&(s=jy(t,a,n,r)),Uy(e),s}function Uy(e){ae.H=vu;var t=we!==null&&we.next!==null;if(Gn=0,Je=we=se=null,pu=!1,io=0,xs=null,t)throw Error(P(300));e===null||ct||(e=e.dependencies,e!==null&&mu(e)&&(ct=!0))}function jy(e,t,a,n){se=e;var r=0;do{if(bs&&(xs=null),io=0,bs=!1,25<=r)throw Error(P(301));if(r+=1,Je=we=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}ae.H=jC,s=t(a,n)}while(bs);return s}function DC(){var e=ae.H,t=e.useState()[0];return t=typeof t.then=="function"?ko(t):t,e=e.useState()[0],(we!==null?we.memoizedState:null)!==e&&(se.flags|=1024),t}function kf(){var e=hu!==0;return hu=0,e}function Rf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function Cf(e){if(pu){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}pu=!1}Gn=0,Je=we=se=null,bs=!1,io=hu=0,xs=null}function Pt(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?se.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(we===null){var e=se.alternate;e=e!==null?e.memoizedState:null}else e=we.next;var t=Je===null?se.memoizedState:Je.next;if(t!==null)Je=t,we=e;else{if(e===null)throw se.alternate===null?Error(P(467)):Error(P(310));we=e,e={memoizedState:we.memoizedState,baseState:we.baseState,baseQueue:we.baseQueue,queue:we.queue,next:null},Je===null?se.memoizedState=Je=e:Je=Je.next=e}return Je}function Ef(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function ko(e){var t=io;return io+=1,xs===null&&(xs=[]),e=Oy(xs,e,t),t=se,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,ae.H=t===null||t.memoizedState===null?fb:pb),e}function qu(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return ko(e);if(e.$$typeof===an)return xt(e)}throw Error(P(438,String(e)))}function Tf(e){var t=null,a=se.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=se.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Ef(),se.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=vR;return t.index++,a}function dn(e,t){return typeof t=="function"?t(e):t}function Jl(e){var t=Xe();return Af(t,we,e)}function Af(e,t,a){var n=e.queue;if(n===null)throw Error(P(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var f=c.lane&-536870913;if(f!==c.lane?(ce&f)===f:(Gn&f)===f){var m=c.revertLane;if(m===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),f===ks&&(d=!0);else if((Gn&m)===m){c=c.next,m===ks&&(d=!0);continue}else f={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,se.lanes|=m,Jn|=m;f=c.action,wr&&a(s,f),s=c.hasEagerState?c.eagerState:a(s,f)}else m={lane:f,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,se.lanes|=f,Jn|=f;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!Wt(s,e.memoizedState)&&(ct=!0,d&&(a=ys,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function Kd(e){var t=Xe(),a=t.queue;if(a===null)throw Error(P(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);Wt(s,t.memoizedState)||(ct=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function Fy(e,t,a){var n=se,r=Xe(),s=fe;if(s){if(a===void 0)throw Error(P(407));a=a()}else a=t();var i=!Wt((we||r).memoizedState,a);i&&(r.memoizedState=a,ct=!0),r=r.queue;var o=By.bind(null,n,r,e);if(Ro(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,Cs(9,zu(),zy.bind(null,n,r,a,t),null),ke===null)throw Error(P(349));s||(Gn&124)!==0||qy(n,t,a)}return a}function qy(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=se.updateQueue,t===null?(t=Ef(),se.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function zy(e,t,a,n){t.value=a,t.getSnapshot=n,Iy(t)&&Ky(e)}function By(e,t,a){return a(function(){Iy(t)&&Ky(e)})}function Iy(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!Wt(e,a)}catch{return!0}}function Ky(e){var t=Ps(e,2);t!==null&&Zt(t,e,2)}function Dm(e){var t=Pt();if(typeof e=="function"){var a=e;if(e=a(),wr){Pn(!0);try{a()}finally{Pn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:dn,lastRenderedState:e},t}function Hy(e,t,a,n){return e.baseState=a,Af(e,we,typeof n=="function"?n:dn)}function MC(e,t,a,n,r){if(Bu(e))throw Error(P(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};ae.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,Qy(t,s)):(s.next=a.next,t.pending=a.next=s)}}function Qy(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=ae.T,i={};ae.T=i;try{var o=a(r,n),u=ae.S;u!==null&&u(i,o),eg(e,t,o)}catch(c){Mm(e,t,c)}finally{ae.T=s}}else try{s=a(r,n),eg(e,t,s)}catch(c){Mm(e,t,c)}}function eg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){tg(e,t,n)},function(n){return Mm(e,t,n)}):tg(e,t,a)}function tg(e,t,a){t.status="fulfilled",t.value=a,Vy(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,Qy(e,a)))}function Mm(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,Vy(t),t=t.next;while(t!==n)}e.action=null}function Vy(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function Gy(e,t){return t}function ag(e,t){if(fe){var a=ke.formState;if(a!==null){e:{var n=se;if(fe){if(Be){t:{for(var r=Be,s=Oa;r.nodeType!==8;){if(!s){r=null;break t}if(r=wa(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){Be=wa(r.nextSibling),n=r.data==="F!";break e}}xr(n)}n=!1}n&&(t=a[0])}}return a=Pt(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:Gy,lastRenderedState:t},a.queue=n,a=cb.bind(null,se,n),n.dispatch=a,n=Dm(!1),s=Lf.bind(null,se,!1,n.queue),n=Pt(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=MC.bind(null,se,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function ng(e){var t=Xe();return Yy(t,we,e)}function Yy(e,t,a){if(t=Af(e,t,Gy)[0],e=Jl(dn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=ko(t)}catch(i){throw i===_o?Fu:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(se.flags|=2048,Cs(9,zu(),OC.bind(null,r,a),null)),[n,s,e]}function OC(e,t){e.action=t}function rg(e){var t=Xe(),a=we;if(a!==null)return Yy(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Cs(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=se.updateQueue,t===null&&(t=Ef(),se.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function zu(){return{destroy:void 0,resource:void 0}}function Jy(){return Xe().memoizedState}function Xl(e,t,a,n){var r=Pt();n=n===void 0?null:n,se.flags|=e,r.memoizedState=Cs(1|t,zu(),a,n)}function Ro(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;we!==null&&n!==null&&Nf(n,we.memoizedState.deps)?r.memoizedState=Cs(t,s,a,n):(se.flags|=e,r.memoizedState=Cs(1|t,s,a,n))}function sg(e,t){Xl(8390656,8,e,t)}function Xy(e,t){Ro(2048,8,e,t)}function Zy(e,t){return Ro(4,2,e,t)}function Wy(e,t){return Ro(4,4,e,t)}function eb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function tb(e,t,a){a=a!=null?a.concat([e]):null,Ro(4,4,eb.bind(null,t,e),a)}function Df(){}function ab(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Nf(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function nb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Nf(t,n[1]))return n[0];if(n=e(),wr){Pn(!0);try{e()}finally{Pn(!1)}}return a.memoizedState=[n,t],n}function Mf(e,t,a){return a===void 0||(Gn&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=Vb(),se.lanes|=e,Jn|=e,a)}function rb(e,t,a,n){return Wt(a,t)?a:Rs.current!==null?(e=Mf(e,a,n),Wt(e,t)||(ct=!0),e):(Gn&42)===0?(ct=!0,e.memoizedState=a):(e=Vb(),se.lanes|=e,Jn|=e,t)}function sb(e,t,a,n,r){var s=pe.p;pe.p=s!==0&&8>s?s:8;var i=ae.T,o={};ae.T=o,Lf(e,!1,t,a);try{var u=r(),c=ae.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=TC(u,n);Yi(e,t,d,Xt(e))}else Yi(e,t,n,Xt(e))}catch(f){Yi(e,t,{then:function(){},status:"rejected",reason:f},Xt())}finally{pe.p=s,ae.T=i}}function LC(){}function Om(e,t,a,n){if(e.tag!==5)throw Error(P(476));var r=ib(e).queue;sb(e,r,t,fr,a===null?LC:function(){return ob(e),a(n)})}function ib(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:fr,baseState:fr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:dn,lastRenderedState:fr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:dn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function ob(e){var t=ib(e).next.queue;Yi(e,t,{},Xt())}function Of(){return xt(mo)}function lb(){return Xe().memoizedState}function ub(){return Xe().memoizedState}function PC(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=Xt();e=zn(a);var n=Bn(t,e,a);n!==null&&(Zt(n,t,a),Qi(n,t,a)),t={cache:xf()},e.payload=t;return}t=t.return}}function UC(e,t,a){var n=Xt();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},Bu(e)?db(t,a):(a=vf(e,t,a,n),a!==null&&(Zt(a,e,n),mb(a,t,n)))}function cb(e,t,a){var n=Xt();Yi(e,t,a,n)}function Yi(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(Bu(e))db(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,Wt(o,i))return ju(e,t,r,0),ke===null&&Uu(),!1}catch{}finally{}if(a=vf(e,t,r,n),a!==null)return Zt(a,e,n),mb(a,t,n),!0}return!1}function Lf(e,t,a,n){if(n={lane:2,revertLane:If(),action:n,hasEagerState:!1,eagerState:null,next:null},Bu(e)){if(t)throw Error(P(479))}else t=vf(e,a,n,2),t!==null&&Zt(t,e,2)}function Bu(e){var t=e.alternate;return e===se||t!==null&&t===se}function db(e,t){bs=pu=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function mb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,ay(e,a)}}var vu={readContext:xt,use:qu,useCallback:He,useContext:He,useEffect:He,useImperativeHandle:He,useLayoutEffect:He,useInsertionEffect:He,useMemo:He,useReducer:He,useRef:He,useState:He,useDebugValue:He,useDeferredValue:He,useTransition:He,useSyncExternalStore:He,useId:He,useHostTransitionStatus:He,useFormState:He,useActionState:He,useOptimistic:He,useMemoCache:He,useCacheRefresh:He},fb={readContext:xt,use:qu,useCallback:function(e,t){return Pt().memoizedState=[e,t===void 0?null:t],e},useContext:xt,useEffect:sg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,Xl(4194308,4,eb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return Xl(4194308,4,e,t)},useInsertionEffect:function(e,t){Xl(4,2,e,t)},useMemo:function(e,t){var a=Pt();t=t===void 0?null:t;var n=e();if(wr){Pn(!0);try{e()}finally{Pn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Pt();if(a!==void 0){var r=a(t);if(wr){Pn(!0);try{a(t)}finally{Pn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=UC.bind(null,se,e),[n.memoizedState,e]},useRef:function(e){var t=Pt();return e={current:e},t.memoizedState=e},useState:function(e){e=Dm(e);var t=e.queue,a=cb.bind(null,se,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:Df,useDeferredValue:function(e,t){var a=Pt();return Mf(a,e,t)},useTransition:function(){var e=Dm(!1);return e=sb.bind(null,se,e.queue,!0,!1),Pt().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=se,r=Pt();if(fe){if(a===void 0)throw Error(P(407));a=a()}else{if(a=t(),ke===null)throw Error(P(349));(ce&124)!==0||qy(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,sg(By.bind(null,n,s,e),[e]),n.flags|=2048,Cs(9,zu(),zy.bind(null,n,s,a,t),null),a},useId:function(){var e=Pt(),t=ke.identifierPrefix;if(fe){var a=rn,n=nn;a=(n&~(1<<32-Jt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=hu++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=AC++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Of,useFormState:ag,useActionState:ag,useOptimistic:function(e){var t=Pt();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Lf.bind(null,se,!0,a),a.dispatch=t,[e,t]},useMemoCache:Tf,useCacheRefresh:function(){return Pt().memoizedState=PC.bind(null,se)}},pb={readContext:xt,use:qu,useCallback:ab,useContext:xt,useEffect:Xy,useImperativeHandle:tb,useInsertionEffect:Zy,useLayoutEffect:Wy,useMemo:nb,useReducer:Jl,useRef:Jy,useState:function(){return Jl(dn)},useDebugValue:Df,useDeferredValue:function(e,t){var a=Xe();return rb(a,we.memoizedState,e,t)},useTransition:function(){var e=Jl(dn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:ko(e),t]},useSyncExternalStore:Fy,useId:lb,useHostTransitionStatus:Of,useFormState:ng,useActionState:ng,useOptimistic:function(e,t){var a=Xe();return Hy(a,we,e,t)},useMemoCache:Tf,useCacheRefresh:ub},jC={readContext:xt,use:qu,useCallback:ab,useContext:xt,useEffect:Xy,useImperativeHandle:tb,useInsertionEffect:Zy,useLayoutEffect:Wy,useMemo:nb,useReducer:Kd,useRef:Jy,useState:function(){return Kd(dn)},useDebugValue:Df,useDeferredValue:function(e,t){var a=Xe();return we===null?Mf(a,e,t):rb(a,we.memoizedState,e,t)},useTransition:function(){var e=Kd(dn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:ko(e),t]},useSyncExternalStore:Fy,useId:lb,useHostTransitionStatus:Of,useFormState:rg,useActionState:rg,useOptimistic:function(e,t){var a=Xe();return we!==null?Hy(a,we,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Tf,useCacheRefresh:ub},$s=null,oo=0;function Pl(e){var t=oo;return oo+=1,$s===null&&($s=[]),Oy($s,e,t)}function Di(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Ul(e,t){throw t.$$typeof===pR?Error(P(525)):(e=Object.prototype.toString.call(t),Error(P(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function ig(e){var t=e._init;return t(e._payload)}function hb(e){function t(g,v){if(e){var x=g.deletions;x===null?(g.deletions=[v],g.flags|=16):x.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=ln(g,v),g.index=0,g.sibling=null,g}function s(g,v,x){return g.index=x,e?(x=g.alternate,x!==null?(x=x.index,x<v?(g.flags|=67108866,v):x):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,x,w){return v===null||v.tag!==6?(v=zd(x,g.mode,w),v.return=g,v):(v=r(v,x),v.return=g,v)}function u(g,v,x,w){var S=x.type;return S===ns?d(g,v,x.props.children,w,x.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Tn&&ig(S)===v.type)?(v=r(v,x.props),Di(v,x),v.return=g,v):(v=Gl(x.type,x.key,x.props,null,g.mode,w),Di(v,x),v.return=g,v)}function c(g,v,x,w){return v===null||v.tag!==4||v.stateNode.containerInfo!==x.containerInfo||v.stateNode.implementation!==x.implementation?(v=Bd(x,g.mode,w),v.return=g,v):(v=r(v,x.children||[]),v.return=g,v)}function d(g,v,x,w,S){return v===null||v.tag!==7?(v=pr(x,g.mode,w,S),v.return=g,v):(v=r(v,x),v.return=g,v)}function f(g,v,x){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=zd(""+v,g.mode,x),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Rl:return x=Gl(v.type,v.key,v.props,null,g.mode,x),Di(x,v),x.return=g,x;case Ui:return v=Bd(v,g.mode,x),v.return=g,v;case Tn:var w=v._init;return v=w(v._payload),f(g,v,x)}if(ji(v)||Ei(v))return v=pr(v,g.mode,x,null),v.return=g,v;if(typeof v.then=="function")return f(g,Pl(v),x);if(v.$$typeof===an)return f(g,Ol(g,v),x);Ul(g,v)}return null}function m(g,v,x,w){var S=v!==null?v.key:null;if(typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint")return S!==null?null:o(g,v,""+x,w);if(typeof x=="object"&&x!==null){switch(x.$$typeof){case Rl:return x.key===S?u(g,v,x,w):null;case Ui:return x.key===S?c(g,v,x,w):null;case Tn:return S=x._init,x=S(x._payload),m(g,v,x,w)}if(ji(x)||Ei(x))return S!==null?null:d(g,v,x,w,null);if(typeof x.then=="function")return m(g,v,Pl(x),w);if(x.$$typeof===an)return m(g,v,Ol(g,x),w);Ul(g,x)}return null}function p(g,v,x,w,S){if(typeof w=="string"&&w!==""||typeof w=="number"||typeof w=="bigint")return g=g.get(x)||null,o(v,g,""+w,S);if(typeof w=="object"&&w!==null){switch(w.$$typeof){case Rl:return g=g.get(w.key===null?x:w.key)||null,u(v,g,w,S);case Ui:return g=g.get(w.key===null?x:w.key)||null,c(v,g,w,S);case Tn:var k=w._init;return w=k(w._payload),p(g,v,x,w,S)}if(ji(w)||Ei(w))return g=g.get(x)||null,d(v,g,w,S,null);if(typeof w.then=="function")return p(g,v,x,Pl(w),S);if(w.$$typeof===an)return p(g,v,x,Ol(v,w),S);Ul(v,w)}return null}function b(g,v,x,w){for(var S=null,k=null,N=v,E=v=0,O=null;N!==null&&E<x.length;E++){N.index>E?(O=N,N=null):O=N.sibling;var L=m(g,N,x[E],w);if(L===null){N===null&&(N=O);break}e&&N&&L.alternate===null&&t(g,N),v=s(L,v,E),k===null?S=L:k.sibling=L,k=L,N=O}if(E===x.length)return a(g,N),fe&&dr(g,E),S;if(N===null){for(;E<x.length;E++)N=f(g,x[E],w),N!==null&&(v=s(N,v,E),k===null?S=N:k.sibling=N,k=N);return fe&&dr(g,E),S}for(N=n(N);E<x.length;E++)O=p(N,g,E,x[E],w),O!==null&&(e&&O.alternate!==null&&N.delete(O.key===null?E:O.key),v=s(O,v,E),k===null?S=O:k.sibling=O,k=O);return e&&N.forEach(function(U){return t(g,U)}),fe&&dr(g,E),S}function y(g,v,x,w){if(x==null)throw Error(P(151));for(var S=null,k=null,N=v,E=v=0,O=null,L=x.next();N!==null&&!L.done;E++,L=x.next()){N.index>E?(O=N,N=null):O=N.sibling;var U=m(g,N,L.value,w);if(U===null){N===null&&(N=O);break}e&&N&&U.alternate===null&&t(g,N),v=s(U,v,E),k===null?S=U:k.sibling=U,k=U,N=O}if(L.done)return a(g,N),fe&&dr(g,E),S;if(N===null){for(;!L.done;E++,L=x.next())L=f(g,L.value,w),L!==null&&(v=s(L,v,E),k===null?S=L:k.sibling=L,k=L);return fe&&dr(g,E),S}for(N=n(N);!L.done;E++,L=x.next())L=p(N,g,E,L.value,w),L!==null&&(e&&L.alternate!==null&&N.delete(L.key===null?E:L.key),v=s(L,v,E),k===null?S=L:k.sibling=L,k=L);return e&&N.forEach(function(D){return t(g,D)}),fe&&dr(g,E),S}function $(g,v,x,w){if(typeof x=="object"&&x!==null&&x.type===ns&&x.key===null&&(x=x.props.children),typeof x=="object"&&x!==null){switch(x.$$typeof){case Rl:e:{for(var S=x.key;v!==null;){if(v.key===S){if(S=x.type,S===ns){if(v.tag===7){a(g,v.sibling),w=r(v,x.props.children),w.return=g,g=w;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Tn&&ig(S)===v.type){a(g,v.sibling),w=r(v,x.props),Di(w,x),w.return=g,g=w;break e}a(g,v);break}else t(g,v);v=v.sibling}x.type===ns?(w=pr(x.props.children,g.mode,w,x.key),w.return=g,g=w):(w=Gl(x.type,x.key,x.props,null,g.mode,w),Di(w,x),w.return=g,g=w)}return i(g);case Ui:e:{for(S=x.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===x.containerInfo&&v.stateNode.implementation===x.implementation){a(g,v.sibling),w=r(v,x.children||[]),w.return=g,g=w;break e}else{a(g,v);break}else t(g,v);v=v.sibling}w=Bd(x,g.mode,w),w.return=g,g=w}return i(g);case Tn:return S=x._init,x=S(x._payload),$(g,v,x,w)}if(ji(x))return b(g,v,x,w);if(Ei(x)){if(S=Ei(x),typeof S!="function")throw Error(P(150));return x=S.call(x),y(g,v,x,w)}if(typeof x.then=="function")return $(g,v,Pl(x),w);if(x.$$typeof===an)return $(g,v,Ol(g,x),w);Ul(g,x)}return typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint"?(x=""+x,v!==null&&v.tag===6?(a(g,v.sibling),w=r(v,x),w.return=g,g=w):(a(g,v),w=zd(x,g.mode,w),w.return=g,g=w),i(g)):a(g,v)}return function(g,v,x,w){try{oo=0;var S=$(g,v,x,w);return $s=null,S}catch(N){if(N===_o||N===Fu)throw N;var k=Gt(29,N,null,g.mode);return k.lanes=w,k.return=g,k}finally{}}}var Es=hb(!0),vb=hb(!1),va=Fa(null),ja=null;function Mn(e){var t=e.alternate;Le(nt,nt.current&1),Le(va,e),ja===null&&(t===null||Rs.current!==null||t.memoizedState!==null)&&(ja=e)}function gb(e){if(e.tag===22){if(Le(nt,nt.current),Le(va,e),ja===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(ja=e)}}else On(e)}function On(){Le(nt,nt.current),Le(va,va.current)}function on(e){dt(va),ja===e&&(ja=null),dt(nt)}var nt=Fa(0);function gu(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||Xm(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function Hd(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Te({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Lm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=Xt(),r=zn(n);r.payload=t,a!=null&&(r.callback=a),t=Bn(e,r,n),t!==null&&(Zt(t,e,n),Qi(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=Xt(),r=zn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Bn(e,r,n),t!==null&&(Zt(t,e,n),Qi(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=Xt(),n=zn(a);n.tag=2,t!=null&&(n.callback=t),t=Bn(e,n,a),t!==null&&(Zt(t,e,a),Qi(t,e,a))}};function og(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!ro(a,n)||!ro(r,s):!0}function lg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Lm.enqueueReplaceState(t,t.state,null)}function Sr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Te({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var yu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function yb(e){yu(e)}function bb(e){console.error(e)}function xb(e){yu(e)}function bu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function ug(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Pm(e,t,a){return a=zn(a),a.tag=3,a.payload={element:null},a.callback=function(){bu(e,t)},a}function $b(e){return e=zn(e),e.tag=3,e}function wb(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){ug(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){ug(t,a,n),typeof r!="function"&&(In===null?In=new Set([this]):In.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function FC(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&So(t,a,r,!0),a=va.current,a!==null){switch(a.tag){case 13:return ja===null?Km():a.alternate===null&&Ie===0&&(Ie=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===Cm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),am(e,n,r)),!1;case 22:return a.flags|=65536,n===Cm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),am(e,n,r)),!1}throw Error(P(435,a.tag))}return am(e,n,r),Km(),!1}if(fe)return t=va.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Sm&&(e=Error(P(422),{cause:n}),so(pa(e,a)))):(n!==Sm&&(t=Error(P(423),{cause:n}),so(pa(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=pa(n,a),r=Pm(e.stateNode,n,r),Id(e,r),Ie!==4&&(Ie=2)),!1;var s=Error(P(520),{cause:n});if(s=pa(s,a),Zi===null?Zi=[s]:Zi.push(s),Ie!==4&&(Ie=2),t===null)return!0;n=pa(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Pm(a.stateNode,n,e),Id(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(In===null||!In.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=$b(r),wb(r,e,a,n),Id(a,r),!1}a=a.return}while(a!==null);return!1}var Sb=Error(P(461)),ct=!1;function ht(e,t,a,n){t.child=e===null?vb(t,null,a,n):Es(t,e.child,a,n)}function cg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return $r(t),n=_f(e,t,a,i,s,r),o=kf(),e!==null&&!ct?(Rf(e,t,r),mn(e,t,r)):(fe&&o&&yf(t),t.flags|=1,ht(e,t,n,r),t.child)}function dg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!gf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Nb(e,t,s,n,r)):(e=Gl(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Pf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:ro,a(i,n)&&e.ref===t.ref)return mn(e,t,r)}return t.flags|=1,e=ln(s,n),e.ref=t.ref,e.return=t,t.child=e}function Nb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(ro(s,n)&&e.ref===t.ref)if(ct=!1,t.pendingProps=n=s,Pf(e,r))(e.flags&131072)!==0&&(ct=!0);else return t.lanes=e.lanes,mn(e,t,r)}return Um(e,t,a,n,r)}function _b(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return mg(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&Yl(t,s!==null?s.cachePool:null),s!==null?Wv(t,s):Am(),gb(t);else return t.lanes=t.childLanes=536870912,mg(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(Yl(t,s.cachePool),Wv(t,s),On(t),t.memoizedState=null):(e!==null&&Yl(t,null),Am(),On(t));return ht(e,t,r,a),t.child}function mg(e,t,a,n){var r=$f();return r=r===null?null:{parent:at._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&Yl(t,null),Am(),gb(t),e!==null&&So(e,t,n,!0),null}function Zl(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(P(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Um(e,t,a,n,r){return $r(t),a=_f(e,t,a,n,void 0,r),n=kf(),e!==null&&!ct?(Rf(e,t,r),mn(e,t,r)):(fe&&n&&yf(t),t.flags|=1,ht(e,t,a,r),t.child)}function fg(e,t,a,n,r,s){return $r(t),t.updateQueue=null,a=jy(t,n,a,r),Uy(e),n=kf(),e!==null&&!ct?(Rf(e,t,s),mn(e,t,s)):(fe&&n&&yf(t),t.flags|=1,ht(e,t,a,s),t.child)}function pg(e,t,a,n,r){if($r(t),t.stateNode===null){var s=ds,i=a.contextType;typeof i=="object"&&i!==null&&(s=xt(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Lm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},wf(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?xt(i):ds,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(Hd(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Lm.enqueueReplaceState(s,s.state,null),Gi(t,n,s,r),Vi(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Sr(a,o);s.props=u;var c=s.context,d=a.contextType;i=ds,typeof d=="object"&&d!==null&&(i=xt(d));var f=a.getDerivedStateFromProps;d=typeof f=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&lg(t,s,n,i),An=!1;var m=t.memoizedState;s.state=m,Gi(t,n,s,r),Vi(),c=t.memoizedState,o||m!==c||An?(typeof f=="function"&&(Hd(t,a,f,n),c=t.memoizedState),(u=An||og(t,a,u,n,m,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Em(e,t),i=t.memoizedProps,d=Sr(a,i),s.props=d,f=t.pendingProps,m=s.context,c=a.contextType,u=ds,typeof c=="object"&&c!==null&&(u=xt(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==f||m!==u)&&lg(t,s,n,u),An=!1,m=t.memoizedState,s.state=m,Gi(t,n,s,r),Vi();var p=t.memoizedState;i!==f||m!==p||An||e!==null&&e.dependencies!==null&&mu(e.dependencies)?(typeof o=="function"&&(Hd(t,a,o,n),p=t.memoizedState),(d=An||og(t,a,d,n,m,p,u)||e!==null&&e.dependencies!==null&&mu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,p,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,p,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=p),s.props=n,s.state=p,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,Zl(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Es(t,e.child,null,r),t.child=Es(t,null,a,r)):ht(e,t,a,r),t.memoizedState=s.state,e=t.child):e=mn(e,t,r),e}function hg(e,t,a,n){return wo(),t.flags|=256,ht(e,t,a,n),t.child}var Qd={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function Vd(e){return{baseLanes:e,cachePool:Dy()}}function Gd(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ha),e}function kb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(nt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(fe){if(r?Mn(t):On(t),fe){var o=Be,u;if(u=o){e:{for(u=o,o=Oa;u.nodeType!==8;){if(!o){o=null;break e}if(u=wa(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:hr!==null?{id:nn,overflow:rn}:null,retryLane:536870912,hydrationErrors:null},u=Gt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,Rt=t,Be=null,u=!0):u=!1}u||xr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return Xm(o)?t.lanes=32:t.lanes=536870912,null;on(t)}return o=n.children,n=n.fallback,r?(On(t),r=t.mode,o=xu({mode:"hidden",children:o},r),n=pr(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=Vd(a),r.childLanes=Gd(e,i,a),t.memoizedState=Qd,n):(Mn(t),jm(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(Mn(t),t.flags&=-257,t=Yd(e,t,a)):t.memoizedState!==null?(On(t),t.child=e.child,t.flags|=128,t=null):(On(t),r=n.fallback,o=t.mode,n=xu({mode:"visible",children:n.children},o),r=pr(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Es(t,e.child,null,a),n=t.child,n.memoizedState=Vd(a),n.childLanes=Gd(e,i,a),t.memoizedState=Qd,t=r);else if(Mn(t),Xm(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(P(419)),n.stack="",n.digest=i,so({value:n,source:null,stack:null}),t=Yd(e,t,a)}else if(ct||So(e,t,a,!1),i=(a&e.childLanes)!==0,ct||i){if(i=ke,i!==null&&(n=a&-a,n=(n&42)!==0?1:sf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,Ps(e,n),Zt(i,e,n),Sb;o.data==="$?"||Km(),t=Yd(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,Be=wa(o.nextSibling),Rt=t,fe=!0,vr=null,Oa=!1,e!==null&&(da[ma++]=nn,da[ma++]=rn,da[ma++]=hr,nn=e.id,rn=e.overflow,hr=t),t=jm(t,n.children),t.flags|=4096);return t}return r?(On(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=ln(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=ln(c,r):(r=pr(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=Vd(a):(u=o.cachePool,u!==null?(c=at._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=Dy(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=Gd(e,i,a),t.memoizedState=Qd,n):(Mn(t),a=e.child,e=a.sibling,a=ln(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function jm(e,t){return t=xu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function xu(e,t){return e=Gt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function Yd(e,t,a){return Es(t,e.child,null,a),e=jm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function vg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),_m(e.return,t,a)}function Jd(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function Rb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(ht(e,t,n.children,a),n=nt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&vg(e,a,t);else if(e.tag===19)vg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Le(nt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&gu(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),Jd(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&gu(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}Jd(t,!0,a,null,s);break;case"together":Jd(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function mn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),Jn|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(So(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(P(153));if(t.child!==null){for(e=t.child,a=ln(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=ln(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Pf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&mu(e)))}function qC(e,t,a){switch(t.tag){case 3:ru(t,t.stateNode.containerInfo),Dn(t,at,e.memoizedState.cache),wo();break;case 27:case 5:fm(t);break;case 4:ru(t,t.stateNode.containerInfo);break;case 10:Dn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(Mn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?kb(e,t,a):(Mn(t),e=mn(e,t,a),e!==null?e.sibling:null);Mn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(So(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return Rb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Le(nt,nt.current),n)break;return null;case 22:case 23:return t.lanes=0,_b(e,t,a);case 24:Dn(t,at,e.memoizedState.cache)}return mn(e,t,a)}function Cb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)ct=!0;else{if(!Pf(e,a)&&(t.flags&128)===0)return ct=!1,qC(e,t,a);ct=(e.flags&131072)!==0}else ct=!1,fe&&(t.flags&1048576)!==0&&Ty(t,du,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")gf(n)?(e=Sr(n,e),t.tag=1,t=pg(null,t,n,e,a)):(t.tag=0,t=Um(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===af){t.tag=11,t=cg(null,t,n,e,a);break e}else if(r===nf){t.tag=14,t=dg(null,t,n,e,a);break e}}throw t=dm(n)||n,Error(P(306,t,""))}}return t;case 0:return Um(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Sr(n,t.pendingProps),pg(e,t,n,r,a);case 3:e:{if(ru(t,t.stateNode.containerInfo),e===null)throw Error(P(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Em(e,t),Gi(t,n,null,a);var i=t.memoizedState;if(n=i.cache,Dn(t,at,n),n!==s.cache&&km(t,[at],a,!0),Vi(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=hg(e,t,n,a);break e}else if(n!==r){r=pa(Error(P(424)),t),so(r),t=hg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(Be=wa(e.firstChild),Rt=t,fe=!0,vr=null,Oa=!0,a=vb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(wo(),n===r){t=mn(e,t,a);break e}ht(e,t,n,a)}t=t.child}return t;case 26:return Zl(e,t),e===null?(a=Lg(t.type,null,t.pendingProps,null))?t.memoizedState=a:fe||(a=t.type,e=t.pendingProps,n=Ru(qn.current).createElement(a),n[bt]=t,n[Ft]=e,gt(n,a,e),ut(n),t.stateNode=n):t.memoizedState=Lg(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return fm(t),e===null&&fe&&(n=t.stateNode=p0(t.type,t.pendingProps,qn.current),Rt=t,Oa=!0,r=Be,Zn(t.type)?(Zm=r,Be=wa(n.firstChild)):Be=r),ht(e,t,t.pendingProps.children,a),Zl(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&fe&&((r=n=Be)&&(n=m3(n,t.type,t.pendingProps,Oa),n!==null?(t.stateNode=n,Rt=t,Be=wa(n.firstChild),Oa=!1,r=!0):r=!1),r||xr(t)),fm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,Ym(r,s)?n=null:i!==null&&Ym(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=_f(e,t,DC,null,null,a),mo._currentValue=r),Zl(e,t),ht(e,t,n,a),t.child;case 6:return e===null&&fe&&((e=a=Be)&&(a=f3(a,t.pendingProps,Oa),a!==null?(t.stateNode=a,Rt=t,Be=null,e=!0):e=!1),e||xr(t)),null;case 13:return kb(e,t,a);case 4:return ru(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Es(t,null,n,a):ht(e,t,n,a),t.child;case 11:return cg(e,t,t.type,t.pendingProps,a);case 7:return ht(e,t,t.pendingProps,a),t.child;case 8:return ht(e,t,t.pendingProps.children,a),t.child;case 12:return ht(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,Dn(t,t.type,n.value),ht(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,$r(t),r=xt(r),n=n(r),t.flags|=1,ht(e,t,n,a),t.child;case 14:return dg(e,t,t.type,t.pendingProps,a);case 15:return Nb(e,t,t.type,t.pendingProps,a);case 19:return Rb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=xu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=ln(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return _b(e,t,a);case 24:return $r(t),n=xt(at),e===null?(r=$f(),r===null&&(r=ke,s=xf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},wf(t),Dn(t,at,r)):((e.lanes&a)!==0&&(Em(e,t),Gi(t,null,null,a),Vi()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),Dn(t,at,n)):(n=s.cache,Dn(t,at,n),n!==r.cache&&km(t,[at],a,!0))),ht(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(P(156,t.tag))}function Wa(e){e.flags|=4}function gg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!g0(t)){if(t=va.current,t!==null&&((ce&4194048)===ce?ja!==null:(ce&62914560)!==ce&&(ce&536870912)===0||t!==ja))throw Hi=Cm,My;e.flags|=8192}}function jl(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?ey():536870912,e.lanes|=t,Ts|=t)}function Mi(e,t){if(!fe)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function je(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function zC(e,t,a){var n=t.pendingProps;switch(bf(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return je(t),null;case 1:return je(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),un(at),Ss(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Ai(t)?Wa(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,Vv())),je(t),null;case 26:return a=t.memoizedState,e===null?(Wa(t),a!==null?(je(t),gg(t,a)):(je(t),t.flags&=-16777217)):a?a!==e.memoizedState?(Wa(t),je(t),gg(t,a)):(je(t),t.flags&=-16777217):(e.memoizedProps!==n&&Wa(t),je(t),t.flags&=-16777217),null;case 27:su(t),a=qn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&Wa(t);else{if(!n){if(t.stateNode===null)throw Error(P(166));return je(t),null}e=Pa.current,Ai(t)?Hv(t,e):(e=p0(r,n,a),t.stateNode=e,Wa(t))}return je(t),null;case 5:if(su(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&Wa(t);else{if(!n){if(t.stateNode===null)throw Error(P(166));return je(t),null}if(e=Pa.current,Ai(t))Hv(t,e);else{switch(r=Ru(qn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[bt]=t,e[Ft]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(gt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&Wa(t)}}return je(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&Wa(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(P(166));if(e=qn.current,Ai(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Rt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[bt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||d0(e.nodeValue,a)),e||xr(t)}else e=Ru(e).createTextNode(n),e[bt]=t,t.stateNode=e}return je(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Ai(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(P(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(P(317));r[bt]=t}else wo(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;je(t),r=!1}else r=Vv(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(on(t),t):(on(t),null)}if(on(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),jl(t,t.updateQueue),je(t),null;case 4:return Ss(),e===null&&Kf(t.stateNode.containerInfo),je(t),null;case 10:return un(t.type),je(t),null;case 19:if(dt(nt),r=t.memoizedState,r===null)return je(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)Mi(r,!1);else{if(Ie!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=gu(e),s!==null){for(t.flags|=128,Mi(r,!1),e=s.updateQueue,t.updateQueue=e,jl(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)Ey(a,e),a=a.sibling;return Le(nt,nt.current&1|2),t.child}e=e.sibling}r.tail!==null&&Ua()>wu&&(t.flags|=128,n=!0,Mi(r,!1),t.lanes=4194304)}else{if(!n)if(e=gu(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,jl(t,e),Mi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!fe)return je(t),null}else 2*Ua()-r.renderingStartTime>wu&&a!==536870912&&(t.flags|=128,n=!0,Mi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=Ua(),t.sibling=null,e=nt.current,Le(nt,n?e&1|2:e&1),t):(je(t),null);case 22:case 23:return on(t),Sf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(je(t),t.subtreeFlags&6&&(t.flags|=8192)):je(t),a=t.updateQueue,a!==null&&jl(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&dt(gr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),un(at),je(t),null;case 25:return null;case 30:return null}throw Error(P(156,t.tag))}function BC(e,t){switch(bf(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return un(at),Ss(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return su(t),null;case 13:if(on(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(P(340));wo()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return dt(nt),null;case 4:return Ss(),null;case 10:return un(t.type),null;case 22:case 23:return on(t),Sf(),e!==null&&dt(gr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return un(at),null;case 25:return null;default:return null}}function Eb(e,t){switch(bf(t),t.tag){case 3:un(at),Ss();break;case 26:case 27:case 5:su(t);break;case 4:Ss();break;case 13:on(t);break;case 19:dt(nt);break;case 10:un(t.type);break;case 22:case 23:on(t),Sf(),e!==null&&dt(gr);break;case 24:un(at)}}function Co(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){Ne(t,t.return,o)}}function Yn(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){Ne(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){Ne(t,t.return,d)}}function Tb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Py(t,a)}catch(n){Ne(e,e.return,n)}}}function Ab(e,t,a){a.props=Sr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){Ne(e,t,n)}}function Ji(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){Ne(e,t,r)}}function La(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){Ne(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){Ne(e,t,r)}else a.current=null}function Db(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){Ne(e,e.return,r)}}function Xd(e,t,a){try{var n=e.stateNode;o3(n,e.type,a,t),n[Ft]=t}catch(r){Ne(e,e.return,r)}}function Mb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&Zn(e.type)||e.tag===4}function Zd(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Mb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&Zn(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Fm(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=Qu));else if(n!==4&&(n===27&&Zn(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Fm(e,t,a),e=e.sibling;e!==null;)Fm(e,t,a),e=e.sibling}function $u(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&Zn(e.type)&&(a=e.stateNode),e=e.child,e!==null))for($u(e,t,a),e=e.sibling;e!==null;)$u(e,t,a),e=e.sibling}function Ob(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);gt(t,n,a),t[bt]=e,t[Ft]=a}catch(s){Ne(e,e.return,s)}}var tn=!1,Qe=!1,Wd=!1,yg=typeof WeakSet=="function"?WeakSet:Set,lt=null;function IC(e,t){if(e=e.containerInfo,Vm=Au,e=$y(e),pf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,f=e,m=null;t:for(;;){for(var p;f!==a||r!==0&&f.nodeType!==3||(o=i+r),f!==s||n!==0&&f.nodeType!==3||(u=i+n),f.nodeType===3&&(i+=f.nodeValue.length),(p=f.firstChild)!==null;)m=f,f=p;for(;;){if(f===e)break t;if(m===a&&++c===r&&(o=i),m===s&&++d===n&&(u=i),(p=f.nextSibling)!==null)break;f=m,m=f.parentNode}f=p}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(Gm={focusedElem:e,selectionRange:a},Au=!1,lt=t;lt!==null;)if(t=lt,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,lt=e;else for(;lt!==null;){switch(t=lt,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var b=Sr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(b,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){Ne(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)Jm(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":Jm(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(P(163))}if(e=t.sibling,e!==null){e.return=t.return,lt=e;break}lt=t.return}}function Lb(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:Cn(e,a),n&4&&Co(5,a);break;case 1:if(Cn(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){Ne(a,a.return,i)}else{var r=Sr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){Ne(a,a.return,i)}}n&64&&Tb(a),n&512&&Ji(a,a.return);break;case 3:if(Cn(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Py(e,t)}catch(i){Ne(a,a.return,i)}}break;case 27:t===null&&n&4&&Ob(a);case 26:case 5:Cn(e,a),t===null&&n&4&&Db(a),n&512&&Ji(a,a.return);break;case 12:Cn(e,a);break;case 13:Cn(e,a),n&4&&jb(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=ZC.bind(null,a),p3(e,a))));break;case 22:if(n=a.memoizedState!==null||tn,!n){t=t!==null&&t.memoizedState!==null||Qe,r=tn;var s=Qe;tn=n,(Qe=t)&&!s?En(e,a,(a.subtreeFlags&8772)!==0):Cn(e,a),tn=r,Qe=s}break;case 30:break;default:Cn(e,a)}}function Pb(e){var t=e.alternate;t!==null&&(e.alternate=null,Pb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&lf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Oe=null,Ut=!1;function en(e,t,a){for(a=a.child;a!==null;)Ub(e,t,a),a=a.sibling}function Ub(e,t,a){if(Yt&&typeof Yt.onCommitFiberUnmount=="function")try{Yt.onCommitFiberUnmount(go,a)}catch{}switch(a.tag){case 26:Qe||La(a,t),en(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Qe||La(a,t);var n=Oe,r=Ut;Zn(a.type)&&(Oe=a.stateNode,Ut=!1),en(e,t,a),eo(a.stateNode),Oe=n,Ut=r;break;case 5:Qe||La(a,t);case 6:if(n=Oe,r=Ut,Oe=null,en(e,t,a),Oe=n,Ut=r,Oe!==null)if(Ut)try{(Oe.nodeType===9?Oe.body:Oe.nodeName==="HTML"?Oe.ownerDocument.body:Oe).removeChild(a.stateNode)}catch(s){Ne(a,t,s)}else try{Oe.removeChild(a.stateNode)}catch(s){Ne(a,t,s)}break;case 18:Oe!==null&&(Ut?(e=Oe,Dg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),ho(e)):Dg(Oe,a.stateNode));break;case 4:n=Oe,r=Ut,Oe=a.stateNode.containerInfo,Ut=!0,en(e,t,a),Oe=n,Ut=r;break;case 0:case 11:case 14:case 15:Qe||Yn(2,a,t),Qe||Yn(4,a,t),en(e,t,a);break;case 1:Qe||(La(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Ab(a,t,n)),en(e,t,a);break;case 21:en(e,t,a);break;case 22:Qe=(n=Qe)||a.memoizedState!==null,en(e,t,a),Qe=n;break;default:en(e,t,a)}}function jb(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{ho(e)}catch(a){Ne(t,t.return,a)}}function KC(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new yg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new yg),t;default:throw Error(P(435,e.tag))}}function em(e,t){var a=KC(e);t.forEach(function(n){var r=WC.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Ht(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(Zn(o.type)){Oe=o.stateNode,Ut=!1;break e}break;case 5:Oe=o.stateNode,Ut=!1;break e;case 3:case 4:Oe=o.stateNode.containerInfo,Ut=!0;break e}o=o.return}if(Oe===null)throw Error(P(160));Ub(s,i,r),Oe=null,Ut=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)Fb(t,e),t=t.sibling}var $a=null;function Fb(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Ht(t,e),Qt(e),n&4&&(Yn(3,e,e.return),Co(3,e),Yn(5,e,e.return));break;case 1:Ht(t,e),Qt(e),n&512&&(Qe||a===null||La(a,a.return)),n&64&&tn&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=$a;if(Ht(t,e),Qt(e),n&512&&(Qe||a===null||La(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[xo]||s[bt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),gt(s,n,a),s[bt]=e,ut(s),n=s;break e;case"link":var i=Ug("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),gt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Ug("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),gt(s,n,a),r.head.appendChild(s);break;default:throw Error(P(468,n))}s[bt]=e,ut(s),n=s}e.stateNode=n}else jg(r,e.type,e.stateNode);else e.stateNode=Pg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?jg(r,e.type,e.stateNode):Pg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&Xd(e,e.memoizedProps,a.memoizedProps)}break;case 27:Ht(t,e),Qt(e),n&512&&(Qe||a===null||La(a,a.return)),a!==null&&n&4&&Xd(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Ht(t,e),Qt(e),n&512&&(Qe||a===null||La(a,a.return)),e.flags&32){r=e.stateNode;try{_s(r,"")}catch(p){Ne(e,e.return,p)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,Xd(e,r,a!==null?a.memoizedProps:r)),n&1024&&(Wd=!0);break;case 6:if(Ht(t,e),Qt(e),n&4){if(e.stateNode===null)throw Error(P(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(p){Ne(e,e.return,p)}}break;case 3:if(tu=null,r=$a,$a=Cu(t.containerInfo),Ht(t,e),$a=r,Qt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{ho(t.containerInfo)}catch(p){Ne(e,e.return,p)}Wd&&(Wd=!1,qb(e));break;case 4:n=$a,$a=Cu(e.stateNode.containerInfo),Ht(t,e),Qt(e),$a=n;break;case 12:Ht(t,e),Qt(e);break;case 13:Ht(t,e),Qt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(zf=Ua()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,em(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=tn,d=Qe;if(tn=c||r,Qe=d||u,Ht(t,e),Qe=d,tn=c,Qt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||tn||Qe||mr(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var f=u.memoizedProps.style,m=f!=null&&f.hasOwnProperty("display")?f.display:null;o.style.display=m==null||typeof m=="boolean"?"":(""+m).trim()}}catch(p){Ne(u,u.return,p)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(p){Ne(u,u.return,p)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,em(e,a))));break;case 19:Ht(t,e),Qt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,em(e,n)));break;case 30:break;case 21:break;default:Ht(t,e),Qt(e)}}function Qt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Mb(n)){a=n;break}n=n.return}if(a==null)throw Error(P(160));switch(a.tag){case 27:var r=a.stateNode,s=Zd(e);$u(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(_s(i,""),a.flags&=-33);var o=Zd(e);$u(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=Zd(e);Fm(e,c,u);break;default:throw Error(P(161))}}catch(d){Ne(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function qb(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;qb(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function Cn(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Lb(e,t.alternate,t),t=t.sibling}function mr(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:Yn(4,t,t.return),mr(t);break;case 1:La(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Ab(t,t.return,a),mr(t);break;case 27:eo(t.stateNode);case 26:case 5:La(t,t.return),mr(t);break;case 22:t.memoizedState===null&&mr(t);break;case 30:mr(t);break;default:mr(t)}e=e.sibling}}function En(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:En(r,s,a),Co(4,s);break;case 1:if(En(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){Ne(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)Ly(u[r],o)}catch(c){Ne(n,n.return,c)}}a&&i&64&&Tb(s),Ji(s,s.return);break;case 27:Ob(s);case 26:case 5:En(r,s,a),a&&n===null&&i&4&&Db(s),Ji(s,s.return);break;case 12:En(r,s,a);break;case 13:En(r,s,a),a&&i&4&&jb(r,s);break;case 22:s.memoizedState===null&&En(r,s,a),Ji(s,s.return);break;case 30:break;default:En(r,s,a)}t=t.sibling}}function Uf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&No(a))}function jf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&No(e))}function Ma(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)zb(e,t,a,n),t=t.sibling}function zb(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ma(e,t,a,n),r&2048&&Co(9,t);break;case 1:Ma(e,t,a,n);break;case 3:Ma(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&No(e)));break;case 12:if(r&2048){Ma(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){Ne(t,t.return,u)}}else Ma(e,t,a,n);break;case 13:Ma(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ma(e,t,a,n):Xi(e,t):s._visibility&2?Ma(e,t,a,n):(s._visibility|=2,ts(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Uf(i,t);break;case 24:Ma(e,t,a,n),r&2048&&jf(t.alternate,t);break;default:Ma(e,t,a,n)}}function ts(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:ts(s,i,o,u,r),Co(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?ts(s,i,o,u,r):Xi(s,i):(d._visibility|=2,ts(s,i,o,u,r)),r&&c&2048&&Uf(i.alternate,i);break;case 24:ts(s,i,o,u,r),r&&c&2048&&jf(i.alternate,i);break;default:ts(s,i,o,u,r)}t=t.sibling}}function Xi(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:Xi(a,n),r&2048&&Uf(n.alternate,n);break;case 24:Xi(a,n),r&2048&&jf(n.alternate,n);break;default:Xi(a,n)}t=t.sibling}}var qi=8192;function Zr(e){if(e.subtreeFlags&qi)for(e=e.child;e!==null;)Bb(e),e=e.sibling}function Bb(e){switch(e.tag){case 26:Zr(e),e.flags&qi&&e.memoizedState!==null&&R3($a,e.memoizedState,e.memoizedProps);break;case 5:Zr(e);break;case 3:case 4:var t=$a;$a=Cu(e.stateNode.containerInfo),Zr(e),$a=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=qi,qi=16777216,Zr(e),qi=t):Zr(e));break;default:Zr(e)}}function Ib(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function Oi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,Hb(n,e)}Ib(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)Kb(e),e=e.sibling}function Kb(e){switch(e.tag){case 0:case 11:case 15:Oi(e),e.flags&2048&&Yn(9,e,e.return);break;case 3:Oi(e);break;case 12:Oi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,Wl(e)):Oi(e);break;default:Oi(e)}}function Wl(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];lt=n,Hb(n,e)}Ib(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:Yn(8,t,t.return),Wl(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,Wl(t));break;default:Wl(t)}e=e.sibling}}function Hb(e,t){for(;lt!==null;){var a=lt;switch(a.tag){case 0:case 11:case 15:Yn(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:No(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,lt=n;else e:for(a=e;lt!==null;){n=lt;var r=n.sibling,s=n.return;if(Pb(n),n===a){lt=null;break e}if(r!==null){r.return=s,lt=r;break e}lt=s}}}var HC={getCacheForType:function(e){var t=xt(at),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},QC=typeof WeakMap=="function"?WeakMap:Map,be=0,ke=null,le=null,ce=0,ye=0,Vt=null,jn=!1,Us=!1,Ff=!1,fn=0,Ie=0,Jn=0,yr=0,qf=0,ha=0,Ts=0,Zi=null,jt=null,qm=!1,zf=0,wu=1/0,Su=null,In=null,vt=0,Kn=null,As=null,ws=0,zm=0,Bm=null,Qb=null,Wi=0,Im=null;function Xt(){if((be&2)!==0&&ce!==0)return ce&-ce;if(ae.T!==null){var e=ks;return e!==0?e:If()}return ny()}function Vb(){ha===0&&(ha=(ce&536870912)===0||fe?Wg():536870912);var e=va.current;return e!==null&&(e.flags|=32),ha}function Zt(e,t,a){(e===ke&&(ye===2||ye===9)||e.cancelPendingCommit!==null)&&(Ds(e,0),Fn(e,ce,ha,!1)),bo(e,a),((be&2)===0||e!==ke)&&(e===ke&&((be&2)===0&&(yr|=a),Ie===4&&Fn(e,ce,ha,!1)),qa(e))}function Gb(e,t,a){if((be&6)!==0)throw Error(P(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||yo(e,t),r=n?YC(e,t):tm(e,t,!0),s=n;do{if(r===0){Us&&!n&&Fn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!VC(a)){r=tm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=Zi;var u=o.current.memoizedState.isDehydrated;if(u&&(Ds(o,i).flags|=256),i=tm(o,i,!1),i!==2){if(Ff&&!u){o.errorRecoveryDisabledLanes|=s,yr|=s,r=4;break e}s=jt,jt=r,s!==null&&(jt===null?jt=s:jt.push.apply(jt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Ds(e,0),Fn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(P(345));case 4:if((t&4194048)!==t)break;case 6:Fn(n,t,ha,!jn);break e;case 2:jt=null;break;case 3:case 5:break;default:throw Error(P(329))}if((t&62914560)===t&&(r=zf+300-Ua(),10<r)){if(Fn(n,t,ha,!jn),Mu(n,0,!0)!==0)break e;n.timeoutHandle=f0(bg.bind(null,n,a,jt,Su,qm,t,ha,yr,Ts,jn,s,2,-0,0),r);break e}bg(n,a,jt,Su,qm,t,ha,yr,Ts,jn,s,0,-0,0)}}break}while(!0);qa(e)}function bg(e,t,a,n,r,s,i,o,u,c,d,f,m,p){if(e.timeoutHandle=-1,f=t.subtreeFlags,(f&8192||(f&16785408)===16785408)&&(co={stylesheets:null,count:0,unsuspend:k3},Bb(t),f=C3(),f!==null)){e.cancelPendingCommit=f($g.bind(null,e,t,s,a,n,r,i,o,u,d,1,m,p)),Fn(e,s,i,!c);return}$g(e,t,s,a,n,r,i,o,u)}function VC(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!Wt(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Fn(e,t,a,n){t&=~qf,t&=~yr,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Jt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&ty(e,a,t)}function Iu(){return(be&6)===0?(Eo(0,!1),!1):!0}function Bf(){if(le!==null){if(ye===0)var e=le.return;else e=le,sn=Rr=null,Cf(e),$s=null,oo=0,e=le;for(;e!==null;)Eb(e.alternate,e),e=e.return;le=null}}function Ds(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,u3(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),Bf(),ke=e,le=a=ln(e.current,null),ce=t,ye=0,Vt=null,jn=!1,Us=yo(e,t),Ff=!1,Ts=ha=qf=yr=Jn=Ie=0,jt=Zi=null,qm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Jt(n),s=1<<r;t|=e[r],n&=~s}return fn=t,Uu(),a}function Yb(e,t){se=null,ae.H=vu,t===_o||t===Fu?(t=Xv(),ye=3):t===My?(t=Xv(),ye=4):ye=t===Sb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Vt=t,le===null&&(Ie=1,bu(e,pa(t,e.current)))}function Jb(){var e=ae.H;return ae.H=vu,e===null?vu:e}function Xb(){var e=ae.A;return ae.A=HC,e}function Km(){Ie=4,jn||(ce&4194048)!==ce&&va.current!==null||(Us=!0),(Jn&134217727)===0&&(yr&134217727)===0||ke===null||Fn(ke,ce,ha,!1)}function tm(e,t,a){var n=be;be|=2;var r=Jb(),s=Xb();(ke!==e||ce!==t)&&(Su=null,Ds(e,t)),t=!1;var i=Ie;e:do try{if(ye!==0&&le!==null){var o=le,u=Vt;switch(ye){case 8:Bf(),i=6;break e;case 3:case 2:case 9:case 6:va.current===null&&(t=!0);var c=ye;if(ye=0,Vt=null,ps(e,o,u,c),a&&Us){i=0;break e}break;default:c=ye,ye=0,Vt=null,ps(e,o,u,c)}}GC(),i=Ie;break}catch(d){Yb(e,d)}while(!0);return t&&e.shellSuspendCounter++,sn=Rr=null,be=n,ae.H=r,ae.A=s,le===null&&(ke=null,ce=0,Uu()),i}function GC(){for(;le!==null;)Zb(le)}function YC(e,t){var a=be;be|=2;var n=Jb(),r=Xb();ke!==e||ce!==t?(Su=null,wu=Ua()+500,Ds(e,t)):Us=yo(e,t);e:do try{if(ye!==0&&le!==null){t=le;var s=Vt;t:switch(ye){case 1:ye=0,Vt=null,ps(e,t,s,1);break;case 2:case 9:if(Jv(s)){ye=0,Vt=null,xg(t);break}t=function(){ye!==2&&ye!==9||ke!==e||(ye=7),qa(e)},s.then(t,t);break e;case 3:ye=7;break e;case 4:ye=5;break e;case 7:Jv(s)?(ye=0,Vt=null,xg(t)):(ye=0,Vt=null,ps(e,t,s,7));break;case 5:var i=null;switch(le.tag){case 26:i=le.memoizedState;case 5:case 27:var o=le;if(!i||g0(i)){ye=0,Vt=null;var u=o.sibling;if(u!==null)le=u;else{var c=o.return;c!==null?(le=c,Ku(c)):le=null}break t}}ye=0,Vt=null,ps(e,t,s,5);break;case 6:ye=0,Vt=null,ps(e,t,s,6);break;case 8:Bf(),Ie=6;break e;default:throw Error(P(462))}}JC();break}catch(d){Yb(e,d)}while(!0);return sn=Rr=null,ae.H=n,ae.A=r,be=a,le!==null?0:(ke=null,ce=0,Uu(),Ie)}function JC(){for(;le!==null&&!yR();)Zb(le)}function Zb(e){var t=Cb(e.alternate,e,fn);e.memoizedProps=e.pendingProps,t===null?Ku(e):le=t}function xg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=fg(a,t,t.pendingProps,t.type,void 0,ce);break;case 11:t=fg(a,t,t.pendingProps,t.type.render,t.ref,ce);break;case 5:Cf(t);default:Eb(a,t),t=le=Ey(t,fn),t=Cb(a,t,fn)}e.memoizedProps=e.pendingProps,t===null?Ku(e):le=t}function ps(e,t,a,n){sn=Rr=null,Cf(t),$s=null,oo=0;var r=t.return;try{if(FC(e,r,t,a,ce)){Ie=1,bu(e,pa(a,e.current)),le=null;return}}catch(s){if(r!==null)throw le=r,s;Ie=1,bu(e,pa(a,e.current)),le=null;return}t.flags&32768?(fe||n===1?e=!0:Us||(ce&536870912)!==0?e=!1:(jn=e=!0,(n===2||n===9||n===3||n===6)&&(n=va.current,n!==null&&n.tag===13&&(n.flags|=16384))),Wb(t,e)):Ku(t)}function Ku(e){var t=e;do{if((t.flags&32768)!==0){Wb(t,jn);return}e=t.return;var a=zC(t.alternate,t,fn);if(a!==null){le=a;return}if(t=t.sibling,t!==null){le=t;return}le=t=e}while(t!==null);Ie===0&&(Ie=5)}function Wb(e,t){do{var a=BC(e.alternate,e);if(a!==null){a.flags&=32767,le=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){le=e;return}le=e=a}while(e!==null);Ie=6,le=null}function $g(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do Hu();while(vt!==0);if((be&6)!==0)throw Error(P(327));if(t!==null){if(t===e.current)throw Error(P(177));if(s=t.lanes|t.childLanes,s|=hf,CR(e,a,s,i,o,u),e===ke&&(le=ke=null,ce=0),As=t,Kn=e,ws=a,zm=s,Bm=r,Qb=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,e3(iu,function(){return r0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=ae.T,ae.T=null,r=pe.p,pe.p=2,i=be,be|=4;try{IC(e,t,a)}finally{be=i,pe.p=r,ae.T=n}}vt=1,e0(),t0(),a0()}}function e0(){if(vt===1){vt=0;var e=Kn,t=As,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=ae.T,ae.T=null;var n=pe.p;pe.p=2;var r=be;be|=4;try{Fb(t,e);var s=Gm,i=$y(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&xy(o.ownerDocument.documentElement,o)){if(u!==null&&pf(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var f=o.ownerDocument||document,m=f&&f.defaultView||window;if(m.getSelection){var p=m.getSelection(),b=o.textContent.length,y=Math.min(u.start,b),$=u.end===void 0?y:Math.min(u.end,b);!p.extend&&y>$&&(i=$,$=y,y=i);var g=Bv(o,y),v=Bv(o,$);if(g&&v&&(p.rangeCount!==1||p.anchorNode!==g.node||p.anchorOffset!==g.offset||p.focusNode!==v.node||p.focusOffset!==v.offset)){var x=f.createRange();x.setStart(g.node,g.offset),p.removeAllRanges(),y>$?(p.addRange(x),p.extend(v.node,v.offset)):(x.setEnd(v.node,v.offset),p.addRange(x))}}}}for(f=[],p=o;p=p.parentNode;)p.nodeType===1&&f.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<f.length;o++){var w=f[o];w.element.scrollLeft=w.left,w.element.scrollTop=w.top}}Au=!!Vm,Gm=Vm=null}finally{be=r,pe.p=n,ae.T=a}}e.current=t,vt=2}}function t0(){if(vt===2){vt=0;var e=Kn,t=As,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=ae.T,ae.T=null;var n=pe.p;pe.p=2;var r=be;be|=4;try{Lb(e,t.alternate,t)}finally{be=r,pe.p=n,ae.T=a}}vt=3}}function a0(){if(vt===4||vt===3){vt=0,bR();var e=Kn,t=As,a=ws,n=Qb;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?vt=5:(vt=0,As=Kn=null,n0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(In=null),of(a),t=t.stateNode,Yt&&typeof Yt.onCommitFiberRoot=="function")try{Yt.onCommitFiberRoot(go,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=ae.T,r=pe.p,pe.p=2,ae.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{ae.T=t,pe.p=r}}(ws&3)!==0&&Hu(),qa(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===Im?Wi++:(Wi=0,Im=e):Wi=0,Eo(0,!1)}}function n0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,No(t)))}function Hu(e){return e0(),t0(),a0(),r0(e)}function r0(){if(vt!==5)return!1;var e=Kn,t=zm;zm=0;var a=of(ws),n=ae.T,r=pe.p;try{pe.p=32>a?32:a,ae.T=null,a=Bm,Bm=null;var s=Kn,i=ws;if(vt=0,As=Kn=null,ws=0,(be&6)!==0)throw Error(P(331));var o=be;if(be|=4,Kb(s.current),zb(s,s.current,i,a),be=o,Eo(0,!1),Yt&&typeof Yt.onPostCommitFiberRoot=="function")try{Yt.onPostCommitFiberRoot(go,s)}catch{}return!0}finally{pe.p=r,ae.T=n,n0(e,t)}}function wg(e,t,a){t=pa(a,t),t=Pm(e.stateNode,t,2),e=Bn(e,t,2),e!==null&&(bo(e,2),qa(e))}function Ne(e,t,a){if(e.tag===3)wg(e,e,a);else for(;t!==null;){if(t.tag===3){wg(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(In===null||!In.has(n))){e=pa(a,e),a=$b(2),n=Bn(t,a,2),n!==null&&(wb(a,n,t,e),bo(n,2),qa(n));break}}t=t.return}}function am(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new QC;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Ff=!0,r.add(a),e=XC.bind(null,e,t,a),t.then(e,e))}function XC(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,ke===e&&(ce&a)===a&&(Ie===4||Ie===3&&(ce&62914560)===ce&&300>Ua()-zf?(be&2)===0&&Ds(e,0):qf|=a,Ts===ce&&(Ts=0)),qa(e)}function s0(e,t){t===0&&(t=ey()),e=Ps(e,t),e!==null&&(bo(e,t),qa(e))}function ZC(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),s0(e,a)}function WC(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(P(314))}n!==null&&n.delete(t),s0(e,a)}function e3(e,t){return rf(e,t)}var Nu=null,as=null,Hm=!1,_u=!1,nm=!1,br=0;function qa(e){e!==as&&e.next===null&&(as===null?Nu=as=e:as=as.next=e),_u=!0,Hm||(Hm=!0,a3())}function Eo(e,t){if(!nm&&_u){nm=!0;do for(var a=!1,n=Nu;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Jt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Sg(n,s))}else s=ce,s=Mu(n,n===ke?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||yo(n,s)||(a=!0,Sg(n,s));n=n.next}while(a);nm=!1}}function t3(){i0()}function i0(){_u=Hm=!1;var e=0;br!==0&&(l3()&&(e=br),br=0);for(var t=Ua(),a=null,n=Nu;n!==null;){var r=n.next,s=o0(n,t);s===0?(n.next=null,a===null?Nu=r:a.next=r,r===null&&(as=a)):(a=n,(e!==0||(s&3)!==0)&&(_u=!0)),n=r}Eo(e,!1)}function o0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Jt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=RR(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=ke,a=ce,a=Mu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&(ye===2||ye===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Td(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||yo(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Td(n),of(a)){case 2:case 8:a=Xg;break;case 32:a=iu;break;case 268435456:a=Zg;break;default:a=iu}return n=l0.bind(null,e),a=rf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Td(n),e.callbackPriority=2,e.callbackNode=null,2}function l0(e,t){if(vt!==0&&vt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(Hu(!0)&&e.callbackNode!==a)return null;var n=ce;return n=Mu(e,e===ke?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(Gb(e,n,t),o0(e,Ua()),e.callbackNode!=null&&e.callbackNode===a?l0.bind(null,e):null)}function Sg(e,t){if(Hu())return null;Gb(e,t,!0)}function a3(){c3(function(){(be&6)!==0?rf(Jg,t3):i0()})}function If(){return br===0&&(br=Wg()),br}function Ng(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:Hl(""+e)}function _g(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function n3(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Ng((r[Ft]||null).action),i=n.submitter;i&&(t=(t=i[Ft]||null)?Ng(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Ou("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(br!==0){var u=i?_g(r,i):new FormData(r);Om(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?_g(r,i):new FormData(r),Om(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(Fl=0;Fl<wm.length;Fl++)ql=wm[Fl],kg=ql.toLowerCase(),Rg=ql[0].toUpperCase()+ql.slice(1),Sa(kg,"on"+Rg);var ql,kg,Rg,Fl;Sa(Sy,"onAnimationEnd");Sa(Ny,"onAnimationIteration");Sa(_y,"onAnimationStart");Sa("dblclick","onDoubleClick");Sa("focusin","onFocus");Sa("focusout","onBlur");Sa(wC,"onTransitionRun");Sa(SC,"onTransitionStart");Sa(NC,"onTransitionCancel");Sa(ky,"onTransitionEnd");Ns("onMouseEnter",["mouseout","mouseover"]);Ns("onMouseLeave",["mouseout","mouseover"]);Ns("onPointerEnter",["pointerout","pointerover"]);Ns("onPointerLeave",["pointerout","pointerover"]);Nr("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Nr("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Nr("onBeforeInput",["compositionend","keypress","textInput","paste"]);Nr("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Nr("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Nr("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var lo="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),r3=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(lo));function u0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){yu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){yu(d)}r.currentTarget=null,s=u}}}}function oe(e,t){var a=t[hm];a===void 0&&(a=t[hm]=new Set);var n=e+"__bubble";a.has(n)||(c0(t,e,2,!1),a.add(n))}function rm(e,t,a){var n=0;t&&(n|=4),c0(a,e,n,t)}var zl="_reactListening"+Math.random().toString(36).slice(2);function Kf(e){if(!e[zl]){e[zl]=!0,ry.forEach(function(a){a!=="selectionchange"&&(r3.has(a)||rm(a,!1,e),rm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[zl]||(t[zl]=!0,rm("selectionchange",!1,t))}}function c0(e,t,a,n){switch(w0(t)){case 2:var r=A3;break;case 8:r=D3;break;default:r=Gf}a=r.bind(null,t,a,e),r=void 0,!bm||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function sm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=ss(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}my(function(){var c=s,d=cf(a),f=[];e:{var m=Ry.get(e);if(m!==void 0){var p=Ou,b=e;switch(e){case"keypress":if(Vl(a)===0)break e;case"keydown":case"keyup":p=eC;break;case"focusin":b="focus",p=jd;break;case"focusout":b="blur",p=jd;break;case"beforeblur":case"afterblur":p=jd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=Mv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=BR;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=nC;break;case Sy:case Ny:case _y:p=HR;break;case ky:p=sC;break;case"scroll":case"scrollend":p=qR;break;case"wheel":p=oC;break;case"copy":case"cut":case"paste":p=VR;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=Lv;break;case"toggle":case"beforetoggle":p=uC}var y=(t&4)!==0,$=!y&&(e==="scroll"||e==="scrollend"),g=y?m!==null?m+"Capture":null:m;y=[];for(var v=c,x;v!==null;){var w=v;if(x=w.stateNode,w=w.tag,w!==5&&w!==26&&w!==27||x===null||g===null||(w=ao(v,g),w!=null&&y.push(uo(v,w,x))),$)break;v=v.return}0<y.length&&(m=new p(m,b,null,a,d),f.push({event:m,listeners:y}))}}if((t&7)===0){e:{if(m=e==="mouseover"||e==="pointerover",p=e==="mouseout"||e==="pointerout",m&&a!==ym&&(b=a.relatedTarget||a.fromElement)&&(ss(b)||b[Os]))break e;if((p||m)&&(m=d.window===d?d:(m=d.ownerDocument)?m.defaultView||m.parentWindow:window,p?(b=a.relatedTarget||a.toElement,p=c,b=b?ss(b):null,b!==null&&($=vo(b),y=b.tag,b!==$||y!==5&&y!==27&&y!==6)&&(b=null)):(p=null,b=c),p!==b)){if(y=Mv,w="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Lv,w="onPointerLeave",g="onPointerEnter",v="pointer"),$=p==null?m:Fi(p),x=b==null?m:Fi(b),m=new y(w,v+"leave",p,a,d),m.target=$,m.relatedTarget=x,w=null,ss(d)===c&&(y=new y(g,v+"enter",b,a,d),y.target=x,y.relatedTarget=$,w=y),$=w,p&&b)t:{for(y=p,g=b,v=0,x=y;x;x=Wr(x))v++;for(x=0,w=g;w;w=Wr(w))x++;for(;0<v-x;)y=Wr(y),v--;for(;0<x-v;)g=Wr(g),x--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=Wr(y),g=Wr(g)}y=null}else y=null;p!==null&&Cg(f,m,p,y,!1),b!==null&&$!==null&&Cg(f,$,b,y,!0)}}e:{if(m=c?Fi(c):window,p=m.nodeName&&m.nodeName.toLowerCase(),p==="select"||p==="input"&&m.type==="file")var S=Fv;else if(jv(m))if(yy)S=bC;else{S=gC;var k=vC}else p=m.nodeName,!p||p.toLowerCase()!=="input"||m.type!=="checkbox"&&m.type!=="radio"?c&&uf(c.elementType)&&(S=Fv):S=yC;if(S&&(S=S(e,c))){gy(f,S,a,d);break e}k&&k(e,m,c),e==="focusout"&&c&&m.type==="number"&&c.memoizedProps.value!=null&&gm(m,"number",m.value)}switch(k=c?Fi(c):window,e){case"focusin":(jv(k)||k.contentEditable==="true")&&(ls=k,xm=c,Ii=null);break;case"focusout":Ii=xm=ls=null;break;case"mousedown":$m=!0;break;case"contextmenu":case"mouseup":case"dragend":$m=!1,Iv(f,a,d);break;case"selectionchange":if($C)break;case"keydown":case"keyup":Iv(f,a,d)}var N;if(ff)e:{switch(e){case"compositionstart":var E="onCompositionStart";break e;case"compositionend":E="onCompositionEnd";break e;case"compositionupdate":E="onCompositionUpdate";break e}E=void 0}else os?hy(e,a)&&(E="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(E="onCompositionStart");E&&(py&&a.locale!=="ko"&&(os||E!=="onCompositionStart"?E==="onCompositionEnd"&&os&&(N=fy()):(Un=d,df="value"in Un?Un.value:Un.textContent,os=!0)),k=ku(c,E),0<k.length&&(E=new Ov(E,e,null,a,d),f.push({event:E,listeners:k}),N?E.data=N:(N=vy(a),N!==null&&(E.data=N)))),(N=dC?mC(e,a):fC(e,a))&&(E=ku(c,"onBeforeInput"),0<E.length&&(k=new Ov("onBeforeInput","beforeinput",null,a,d),f.push({event:k,listeners:E}),k.data=N)),n3(f,e,c,a,d)}u0(f,t)})}function uo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function ku(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=ao(e,a),r!=null&&n.unshift(uo(e,r,s)),r=ao(e,t),r!=null&&n.push(uo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function Wr(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Cg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=ao(a,s),c!=null&&i.unshift(uo(a,c,u))):r||(c=ao(a,s),c!=null&&i.push(uo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var s3=/\r\n?/g,i3=/\u0000|\uFFFD/g;function Eg(e){return(typeof e=="string"?e:""+e).replace(s3,`
`).replace(i3,"")}function d0(e,t){return t=Eg(t),Eg(e)===t}function Qu(){}function $e(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||_s(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&_s(e,""+n);break;case"className":Tl(e,"class",n);break;case"tabIndex":Tl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Tl(e,a,n);break;case"style":dy(e,n,s);break;case"data":if(t!=="object"){Tl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Hl(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&$e(e,t,"name",r.name,r,null),$e(e,t,"formEncType",r.formEncType,r,null),$e(e,t,"formMethod",r.formMethod,r,null),$e(e,t,"formTarget",r.formTarget,r,null)):($e(e,t,"encType",r.encType,r,null),$e(e,t,"method",r.method,r,null),$e(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=Hl(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=Qu);break;case"onScroll":n!=null&&oe("scroll",e);break;case"onScrollEnd":n!=null&&oe("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(P(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(P(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=Hl(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":oe("beforetoggle",e),oe("toggle",e),Kl(e,"popover",n);break;case"xlinkActuate":Za(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":Za(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":Za(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":Za(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":Za(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":Za(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":Za(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":Za(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":Za(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":Kl(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=jR.get(a)||a,Kl(e,a,n))}}function Qm(e,t,a,n,r,s){switch(a){case"style":dy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(P(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(P(60));e.innerHTML=a}}break;case"children":typeof n=="string"?_s(e,n):(typeof n=="number"||typeof n=="bigint")&&_s(e,""+n);break;case"onScroll":n!=null&&oe("scroll",e);break;case"onScrollEnd":n!=null&&oe("scrollend",e);break;case"onClick":n!=null&&(e.onclick=Qu);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!sy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[Ft]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):Kl(e,a,n)}}}function gt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":oe("error",e),oe("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(P(137,t));default:$e(e,t,s,i,a,null)}}r&&$e(e,t,"srcSet",a.srcSet,a,null),n&&$e(e,t,"src",a.src,a,null);return;case"input":oe("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(P(137,t));break;default:$e(e,t,n,d,a,null)}}ly(e,s,o,u,c,i,r,!1),ou(e);return;case"select":oe("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:$e(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?vs(e,!!n,t,!1):a!=null&&vs(e,!!n,a,!0);return;case"textarea":oe("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(P(91));break;default:$e(e,t,i,o,a,null)}cy(e,n,r,s),ou(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:$e(e,t,u,n,a,null)}return;case"dialog":oe("beforetoggle",e),oe("toggle",e),oe("cancel",e),oe("close",e);break;case"iframe":case"object":oe("load",e);break;case"video":case"audio":for(n=0;n<lo.length;n++)oe(lo[n],e);break;case"image":oe("error",e),oe("load",e);break;case"details":oe("toggle",e);break;case"embed":case"source":case"link":oe("error",e),oe("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(P(137,t));default:$e(e,t,c,n,a,null)}return;default:if(uf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&Qm(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&$e(e,t,o,n,a,null))}function o3(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(p in a){var f=a[p];if(a.hasOwnProperty(p)&&f!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":u=f;default:n.hasOwnProperty(p)||$e(e,t,p,null,n,f)}}for(var m in n){var p=n[m];if(f=a[m],n.hasOwnProperty(m)&&(p!=null||f!=null))switch(m){case"type":s=p;break;case"name":r=p;break;case"checked":c=p;break;case"defaultChecked":d=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(P(137,t));break;default:p!==f&&$e(e,t,m,p,n,f)}}vm(e,i,o,u,c,d,s,r);return;case"select":p=i=o=m=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":p=u;default:n.hasOwnProperty(s)||$e(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":m=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&$e(e,t,r,s,n,u)}t=o,a=i,n=p,m!=null?vs(e,!!a,m,!1):!!n!=!!a&&(t!=null?vs(e,!!a,t,!0):vs(e,!!a,a?[]:"",!1));return;case"textarea":p=m=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:$e(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":m=r;break;case"defaultValue":p=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(P(91));break;default:r!==s&&$e(e,t,i,r,n,s)}uy(e,m,p);return;case"option":for(var b in a)if(m=a[b],a.hasOwnProperty(b)&&m!=null&&!n.hasOwnProperty(b))switch(b){case"selected":e.selected=!1;break;default:$e(e,t,b,null,n,m)}for(u in n)if(m=n[u],p=a[u],n.hasOwnProperty(u)&&m!==p&&(m!=null||p!=null))switch(u){case"selected":e.selected=m&&typeof m!="function"&&typeof m!="symbol";break;default:$e(e,t,u,m,n,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)m=a[y],a.hasOwnProperty(y)&&m!=null&&!n.hasOwnProperty(y)&&$e(e,t,y,null,n,m);for(c in n)if(m=n[c],p=a[c],n.hasOwnProperty(c)&&m!==p&&(m!=null||p!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(m!=null)throw Error(P(137,t));break;default:$e(e,t,c,m,n,p)}return;default:if(uf(t)){for(var $ in a)m=a[$],a.hasOwnProperty($)&&m!==void 0&&!n.hasOwnProperty($)&&Qm(e,t,$,void 0,n,m);for(d in n)m=n[d],p=a[d],!n.hasOwnProperty(d)||m===p||m===void 0&&p===void 0||Qm(e,t,d,m,n,p);return}}for(var g in a)m=a[g],a.hasOwnProperty(g)&&m!=null&&!n.hasOwnProperty(g)&&$e(e,t,g,null,n,m);for(f in n)m=n[f],p=a[f],!n.hasOwnProperty(f)||m===p||m==null&&p==null||$e(e,t,f,m,n,p)}var Vm=null,Gm=null;function Ru(e){return e.nodeType===9?e:e.ownerDocument}function Tg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function m0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function Ym(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var im=null;function l3(){var e=window.event;return e&&e.type==="popstate"?e===im?!1:(im=e,!0):(im=null,!1)}var f0=typeof setTimeout=="function"?setTimeout:void 0,u3=typeof clearTimeout=="function"?clearTimeout:void 0,Ag=typeof Promise=="function"?Promise:void 0,c3=typeof queueMicrotask=="function"?queueMicrotask:typeof Ag<"u"?function(e){return Ag.resolve(null).then(e).catch(d3)}:f0;function d3(e){setTimeout(function(){throw e})}function Zn(e){return e==="head"}function Dg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&eo(i.documentElement),a&2&&eo(i.body),a&4)for(a=i.head,eo(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[xo]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),ho(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);ho(t)}function Jm(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":Jm(a),lf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function m3(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[xo])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=wa(e.nextSibling),e===null)break}return null}function f3(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=wa(e.nextSibling),e===null))return null;return e}function Xm(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function p3(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function wa(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var Zm=null;function Mg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function p0(e,t,a){switch(t=Ru(a),e){case"html":if(e=t.documentElement,!e)throw Error(P(452));return e;case"head":if(e=t.head,!e)throw Error(P(453));return e;case"body":if(e=t.body,!e)throw Error(P(454));return e;default:throw Error(P(451))}}function eo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);lf(e)}var ga=new Map,Og=new Set;function Cu(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var pn=pe.d;pe.d={f:h3,r:v3,D:g3,C:y3,L:b3,m:x3,X:w3,S:$3,M:S3};function h3(){var e=pn.f(),t=Iu();return e||t}function v3(e){var t=Ls(e);t!==null&&t.tag===5&&t.type==="form"?ob(t):pn.r(e)}var js=typeof document>"u"?null:document;function h0(e,t,a){var n=js;if(n&&typeof t=="string"&&t){var r=fa(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Og.has(r)||(Og.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),gt(t,"link",e),ut(t),n.head.appendChild(t)))}}function g3(e){pn.D(e),h0("dns-prefetch",e,null)}function y3(e,t){pn.C(e,t),h0("preconnect",e,t)}function b3(e,t,a){pn.L(e,t,a);var n=js;if(n&&e&&t){var r='link[rel="preload"][as="'+fa(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+fa(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+fa(a.imageSizes)+'"]')):r+='[href="'+fa(e)+'"]';var s=r;switch(t){case"style":s=Ms(e);break;case"script":s=Fs(e)}ga.has(s)||(e=Te({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),ga.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(To(s))||t==="script"&&n.querySelector(Ao(s))||(t=n.createElement("link"),gt(t,"link",e),ut(t),n.head.appendChild(t)))}}function x3(e,t){pn.m(e,t);var a=js;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+fa(n)+'"][href="'+fa(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Fs(e)}if(!ga.has(s)&&(e=Te({rel:"modulepreload",href:e},t),ga.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Ao(s)))return}n=a.createElement("link"),gt(n,"link",e),ut(n),a.head.appendChild(n)}}}function $3(e,t,a){pn.S(e,t,a);var n=js;if(n&&e){var r=hs(n).hoistableStyles,s=Ms(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(To(s)))o.loading=5;else{e=Te({rel:"stylesheet",href:e,"data-precedence":t},a),(a=ga.get(s))&&Hf(e,a);var u=i=n.createElement("link");ut(u),gt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,eu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function w3(e,t){pn.X(e,t);var a=js;if(a&&e){var n=hs(a).hoistableScripts,r=Fs(e),s=n.get(r);s||(s=a.querySelector(Ao(r)),s||(e=Te({src:e,async:!0},t),(t=ga.get(r))&&Qf(e,t),s=a.createElement("script"),ut(s),gt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function S3(e,t){pn.M(e,t);var a=js;if(a&&e){var n=hs(a).hoistableScripts,r=Fs(e),s=n.get(r);s||(s=a.querySelector(Ao(r)),s||(e=Te({src:e,async:!0,type:"module"},t),(t=ga.get(r))&&Qf(e,t),s=a.createElement("script"),ut(s),gt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Lg(e,t,a,n){var r=(r=qn.current)?Cu(r):null;if(!r)throw Error(P(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=Ms(a.href),a=hs(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=Ms(a.href);var s=hs(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(To(e)))&&!s._p&&(i.instance=s,i.state.loading=5),ga.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},ga.set(e,a),s||N3(r,e,a,i.state))),t&&n===null)throw Error(P(528,""));return i}if(t&&n!==null)throw Error(P(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Fs(a),a=hs(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(P(444,e))}}function Ms(e){return'href="'+fa(e)+'"'}function To(e){return'link[rel="stylesheet"]['+e+"]"}function v0(e){return Te({},e,{"data-precedence":e.precedence,precedence:null})}function N3(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),gt(t,"link",a),ut(t),e.head.appendChild(t))}function Fs(e){return'[src="'+fa(e)+'"]'}function Ao(e){return"script[async]"+e}function Pg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+fa(a.href)+'"]');if(n)return t.instance=n,ut(n),n;var r=Te({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ut(n),gt(n,"style",r),eu(n,a.precedence,e),t.instance=n;case"stylesheet":r=Ms(a.href);var s=e.querySelector(To(r));if(s)return t.state.loading|=4,t.instance=s,ut(s),s;n=v0(a),(r=ga.get(r))&&Hf(n,r),s=(e.ownerDocument||e).createElement("link"),ut(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),gt(s,"link",n),t.state.loading|=4,eu(s,a.precedence,e),t.instance=s;case"script":return s=Fs(a.src),(r=e.querySelector(Ao(s)))?(t.instance=r,ut(r),r):(n=a,(r=ga.get(s))&&(n=Te({},a),Qf(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ut(r),gt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(P(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,eu(n,a.precedence,e));return t.instance}function eu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function Hf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function Qf(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var tu=null;function Ug(e,t,a){if(tu===null){var n=new Map,r=tu=new Map;r.set(a,n)}else r=tu,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[xo]||s[bt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function jg(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function _3(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function g0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var co=null;function k3(){}function R3(e,t,a){if(co===null)throw Error(P(475));var n=co;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=Ms(a.href),s=e.querySelector(To(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Eu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ut(s);return}s=e.ownerDocument||e,a=v0(a),(r=ga.get(r))&&Hf(a,r),s=s.createElement("link"),ut(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),gt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Eu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function C3(){if(co===null)throw Error(P(475));var e=co;return e.stylesheets&&e.count===0&&Wm(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&Wm(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Eu(){if(this.count--,this.count===0){if(this.stylesheets)Wm(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Tu=null;function Wm(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Tu=new Map,t.forEach(E3,e),Tu=null,Eu.call(e))}function E3(e,t){if(!(t.state.loading&4)){var a=Tu.get(e);if(a)var n=a.get(null);else{a=new Map,Tu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Eu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var mo={$$typeof:an,Provider:null,Consumer:null,_currentValue:fr,_currentValue2:fr,_threadCount:0};function T3(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=Ad(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=Ad(0),this.hiddenUpdates=Ad(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function y0(e,t,a,n,r,s,i,o,u,c,d,f){return e=new T3(e,t,a,i,o,u,c,f),t=1,s===!0&&(t|=24),s=Gt(3,null,null,t),e.current=s,s.stateNode=e,t=xf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},wf(s),e}function b0(e){return e?(e=ds,e):ds}function x0(e,t,a,n,r,s){r=b0(r),n.context===null?n.context=r:n.pendingContext=r,n=zn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Bn(e,n,t),a!==null&&(Zt(a,e,t),Qi(a,e,t))}function Fg(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function Vf(e,t){Fg(e,t),(e=e.alternate)&&Fg(e,t)}function $0(e){if(e.tag===13){var t=Ps(e,67108864);t!==null&&Zt(t,e,67108864),Vf(e,67108864)}}var Au=!0;function A3(e,t,a,n){var r=ae.T;ae.T=null;var s=pe.p;try{pe.p=2,Gf(e,t,a,n)}finally{pe.p=s,ae.T=r}}function D3(e,t,a,n){var r=ae.T;ae.T=null;var s=pe.p;try{pe.p=8,Gf(e,t,a,n)}finally{pe.p=s,ae.T=r}}function Gf(e,t,a,n){if(Au){var r=ef(n);if(r===null)sm(e,t,n,Du,a),qg(e,n);else if(O3(r,e,t,a,n))n.stopPropagation();else if(qg(e,n),t&4&&-1<M3.indexOf(e)){for(;r!==null;){var s=Ls(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=cr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Jt(i);o.entanglements[1]|=u,i&=~u}qa(s),(be&6)===0&&(wu=Ua()+500,Eo(0,!1))}}break;case 13:o=Ps(s,2),o!==null&&Zt(o,s,2),Iu(),Vf(s,2)}if(s=ef(n),s===null&&sm(e,t,n,Du,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else sm(e,t,n,null,a)}}function ef(e){return e=cf(e),Yf(e)}var Du=null;function Yf(e){if(Du=null,e=ss(e),e!==null){var t=vo(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=Qg(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return Du=e,null}function w0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(xR()){case Jg:return 2;case Xg:return 8;case iu:case $R:return 32;case Zg:return 268435456;default:return 32}default:return 32}}var tf=!1,Hn=null,Qn=null,Vn=null,fo=new Map,po=new Map,Ln=[],M3="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function qg(e,t){switch(e){case"focusin":case"focusout":Hn=null;break;case"dragenter":case"dragleave":Qn=null;break;case"mouseover":case"mouseout":Vn=null;break;case"pointerover":case"pointerout":fo.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":po.delete(t.pointerId)}}function Li(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Ls(t),t!==null&&$0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function O3(e,t,a,n,r){switch(t){case"focusin":return Hn=Li(Hn,e,t,a,n,r),!0;case"dragenter":return Qn=Li(Qn,e,t,a,n,r),!0;case"mouseover":return Vn=Li(Vn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return fo.set(s,Li(fo.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,po.set(s,Li(po.get(s)||null,e,t,a,n,r)),!0}return!1}function S0(e){var t=ss(e.target);if(t!==null){var a=vo(t);if(a!==null){if(t=a.tag,t===13){if(t=Qg(a),t!==null){e.blockedOn=t,ER(e.priority,function(){if(a.tag===13){var n=Xt();n=sf(n);var r=Ps(a,n);r!==null&&Zt(r,a,n),Vf(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function au(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=ef(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);ym=n,a.target.dispatchEvent(n),ym=null}else return t=Ls(a),t!==null&&$0(t),e.blockedOn=a,!1;t.shift()}return!0}function zg(e,t,a){au(e)&&a.delete(t)}function L3(){tf=!1,Hn!==null&&au(Hn)&&(Hn=null),Qn!==null&&au(Qn)&&(Qn=null),Vn!==null&&au(Vn)&&(Vn=null),fo.forEach(zg),po.forEach(zg)}function Bl(e,t){e.blockedOn===t&&(e.blockedOn=null,tf||(tf=!0,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,L3)))}var Il=null;function Bg(e){Il!==e&&(Il=e,rt.unstable_scheduleCallback(rt.unstable_NormalPriority,function(){Il===e&&(Il=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(Yf(n||a)===null)continue;break}var s=Ls(a);s!==null&&(e.splice(t,3),t-=3,Om(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function ho(e){function t(u){return Bl(u,e)}Hn!==null&&Bl(Hn,e),Qn!==null&&Bl(Qn,e),Vn!==null&&Bl(Vn,e),fo.forEach(t),po.forEach(t);for(var a=0;a<Ln.length;a++){var n=Ln[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<Ln.length&&(a=Ln[0],a.blockedOn===null);)S0(a),a.blockedOn===null&&Ln.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[Ft]||null;if(typeof s=="function")i||Bg(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[Ft]||null)o=i.formAction;else if(Yf(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),Bg(a)}}}function Jf(e){this._internalRoot=e}Vu.prototype.render=Jf.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(P(409));var a=t.current,n=Xt();x0(a,n,e,t,null,null)};Vu.prototype.unmount=Jf.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;x0(e.current,2,null,e,null,null),Iu(),t[Os]=null}};function Vu(e){this._internalRoot=e}Vu.prototype.unstable_scheduleHydration=function(e){if(e){var t=ny();e={blockedOn:null,target:e,priority:t};for(var a=0;a<Ln.length&&t!==0&&t<Ln[a].priority;a++);Ln.splice(a,0,e),a===0&&S0(e)}};var Ig=Kg.version;if(Ig!=="19.1.0")throw Error(P(527,Ig,"19.1.0"));pe.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(P(188)):(e=Object.keys(e).join(","),Error(P(268,e)));return e=fR(t),e=e!==null?Vg(e):null,e=e===null?null:e.stateNode,e};var P3={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:ae,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Pi=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Pi.isDisabled&&Pi.supportsFiber))try{go=Pi.inject(P3),Yt=Pi}catch{}var Pi;Gu.createRoot=function(e,t){if(!Hg(e))throw Error(P(299));var a=!1,n="",r=yb,s=bb,i=xb,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=y0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Os]=t.current,Kf(e),new Jf(t)};Gu.hydrateRoot=function(e,t,a){if(!Hg(e))throw Error(P(299));var n=!1,r="",s=yb,i=bb,o=xb,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=y0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=b0(null),a=t.current,n=Xt(),n=sf(n),r=zn(n),r.callback=null,Bn(a,r,n),a=n,t.current.lanes=a,bo(t,a),qa(t),e[Os]=t.current,Kf(e),new Vu(t)};Gu.version="19.1.0"});var R0=Sn((JL,k0)=>{"use strict";function _0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(_0)}catch(e){console.error(e)}}_0(),k0.exports=N0()});var Mt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var Qk={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},Vk=class{#t=Qk;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ea=new Vk;function Ph(e){setTimeout(e,0)}var Ot=typeof window>"u"||"Deno"in globalThis;function De(){}function Fh(e,t){return typeof e=="function"?e(t):e}function gi(e){return typeof e=="number"&&e>=0&&e!==1/0}function il(e,t){return Math.max(e+(t||0)-Date.now(),0)}function xa(e,t){return typeof e=="function"?e(t):e}function Lt(e,t){return typeof e=="function"?e(t):e}function ol(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==yi(i,t.options))return!1}else if(!or(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function ll(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Ta(t.options.mutationKey)!==Ta(s))return!1}else if(!or(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function yi(e,t){return(t?.queryKeyHashFn||Ta)(e)}function Ta(e){return JSON.stringify(e,(t,a)=>ld(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function or(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>or(e[a],t[a])):!1}var Gk=Object.prototype.hasOwnProperty;function bi(e,t){if(e===t)return e;let a=Uh(e)&&Uh(t);if(!a&&!(ld(e)&&ld(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],f=e[d],m=t[d];if(f===m){o[d]=f,(a?c<r:Gk.call(e,d))&&u++;continue}if(f===null||m===null||typeof f!="object"||typeof m!="object"){o[d]=m;continue}let p=bi(f,m);o[d]=p,p===f&&u++}return r===i&&u===r?e:o}function Nn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Uh(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function ld(e){if(!jh(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!jh(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function jh(e){return Object.prototype.toString.call(e)==="[object Object]"}function qh(e){return new Promise(t=>{Ea.setTimeout(t,e)})}function xi(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?bi(e,t):t}function zh(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function Bh(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var Kr=Symbol();function ul(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===Kr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function $i(e,t){return typeof e=="function"?e(...t):!!e}var Yk=class extends Mt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ot&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},Hr=new Yk;function wi(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var Ih=Ph;function Jk(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=Ih,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var ue=Jk();var Xk=class extends Mt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ot&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},Qr=new Xk;function Zk(e){return Math.min(1e3*2**e,3e4)}function ud(e){return(e??"online")==="online"?Qr.isOnline():!0}var cl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function dl(e){let t=!1,a=0,n,r=wi(),s=()=>r.status!=="pending",i=y=>{if(!s()){let $=new cl(y);m($),e.onCancel?.($)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>Hr.isFocused()&&(e.networkMode==="always"||Qr.isOnline())&&e.canRun(),d=()=>ud(e.networkMode)&&e.canRun(),f=y=>{s()||(n?.(),r.resolve(y))},m=y=>{s()||(n?.(),r.reject(y))},p=()=>new Promise(y=>{n=$=>{(s()||c())&&y($)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),b=()=>{if(s())return;let y,$=a===0?e.initialPromise:void 0;try{y=$??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(f).catch(g=>{if(s())return;let v=e.retry??(Ot?0:3),x=e.retryDelay??Zk,w=typeof x=="function"?x(a,g):x,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){m(g);return}a++,e.onFail?.(a,g),qh(w).then(()=>c()?void 0:p()).then(()=>{t?m(g):b()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?b():p().then(b),r)}}var ml=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),gi(this.gcTime)&&(this.#t=Ea.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ot?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ea.clearTimeout(this.#t),this.#t=void 0)}};var Hh=class extends ml{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=Kh(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=Kh(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=xi(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(De).catch(De):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>Lt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===Kr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>xa(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!il(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=ul(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=dl({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof cl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof cl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...cd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),ue.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function cd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:ud(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function Kh(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var lr=class extends Mt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=wi(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),Qh(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return dd(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return dd(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof Lt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Nn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&Vh(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||Lt(this.options.enabled,this.#e)!==Lt(t.enabled,this.#e)||xa(this.options.staleTime,this.#e)!==xa(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||Lt(this.options.enabled,this.#e)!==Lt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return eR(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(De)),t}#v(){this.#x();let e=xa(this.options.staleTime,this.#e);if(Ot||this.#n.isStale||!gi(e))return;let a=il(this.#n.dataUpdatedAt,e)+1;this.#u=Ea.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ot||Lt(this.options.enabled,this.#e)===!1||!gi(this.#l)||this.#l===0)&&(this.#c=Ea.setInterval(()=>{(this.options.refetchIntervalInBackground||Hr.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ea.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ea.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},f=!1,m;if(t._optimisticResults){let E=this.hasListeners(),O=!E&&Qh(e,t),L=E&&Vh(e,a,t,n);(O||L)&&(d={...d,...cd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:p,errorUpdatedAt:b,status:y}=d;m=d.data;let $=!1;if(t.placeholderData!==void 0&&m===void 0&&y==="pending"){let E;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(E=r.data,$=!0):E=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,E!==void 0&&(y="success",m=xi(r?.data,E,t),f=!0)}if(t.select&&m!==void 0&&!$)if(r&&m===s?.data&&t.select===this.#f)m=this.#d;else try{this.#f=t.select,m=t.select(m),m=xi(r?.data,m,t),this.#d=m,this.#i=null}catch(E){this.#i=E}this.#i&&(p=this.#i,m=this.#d,b=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",x=y==="error",w=v&&g,S=m!==void 0,N={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:x,isInitialLoading:w,isLoading:w,data:m,dataUpdatedAt:d.dataUpdatedAt,error:p,errorUpdatedAt:b,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:x&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:f,isRefetchError:x&&S,isStale:md(e,t),refetch:this.refetch,promise:this.#o,isEnabled:Lt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let E=U=>{N.status==="error"?U.reject(N.error):N.data!==void 0&&U.resolve(N.data)},O=()=>{let U=this.#o=N.promise=wi();E(U)},L=this.#o;switch(L.status){case"pending":e.queryHash===a.queryHash&&E(L);break;case"fulfilled":(N.status==="error"||N.data!==L.value)&&O();break;case"rejected":(N.status!=="error"||N.error!==L.reason)&&O();break}}return N}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Nn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){ue.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function Wk(e,t){return Lt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function Qh(e,t){return Wk(e,t)||e.state.data!==void 0&&dd(e,t,t.refetchOnMount)}function dd(e,t,a){if(Lt(t.enabled,e)!==!1&&xa(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&md(e,t)}return!1}function Vh(e,t,a,n){return(e!==t||Lt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&md(e,a)}function md(e,t){return Lt(t.enabled,e)!==!1&&e.isStaleByTime(xa(t.staleTime,e))}function eR(e,t){return!Nn(e.getCurrentResult(),t)}function fd(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,f=b=>{Object.defineProperty(b,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},m=ul(t.options,t.fetchOptions),p=async(b,y,$)=>{if(d)return Promise.reject();if(y==null&&b.pages.length)return Promise.resolve(b);let v=(()=>{let k={client:t.client,queryKey:t.queryKey,pageParam:y,direction:$?"backward":"forward",meta:t.options.meta};return f(k),k})(),x=await m(v),{maxPages:w}=t.options,S=$?Bh:zh;return{pages:S(b.pages,x,w),pageParams:S(b.pageParams,y,w)}};if(r&&s.length){let b=r==="backward",y=b?tR:Gh,$={pages:s,pageParams:i},g=y(n,$);o=await p($,g,b)}else{let b=e??s.length;do{let y=u===0?i[0]??n.initialPageParam:Gh(n,o);if(u>0&&y==null)break;o=await p(o,y),u++}while(u<b)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function Gh(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function tR(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var Yh=class extends ml{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||pd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=dl({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),ue.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function pd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var Jh=class extends Mt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new Yh({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=fl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=fl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=fl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=fl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){ue.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>ll(t,a))}findAll(e={}){return this.getAll().filter(t=>ll(e,t))}notify(e){ue.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return ue.batch(()=>Promise.all(e.map(t=>t.continue().catch(De))))}};function fl(e){return e.options.scope?.id}var hd=class extends Mt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Nn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Ta(t.mutationKey)!==Ta(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??pd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){ue.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function Xh(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function aR(e,t,a){let n=e.slice(0);return n[t]=a,n}var vd=class extends Mt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,ue.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,f)=>d!==a[f]),u=i||o,c=u?!0:s.some((d,f)=>{let m=this.#e[f];return!m||!Nn(d,m)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(Xh(a,r).forEach(d=>{d.destroy()}),Xh(r,a).forEach(d=>{d.subscribe(f=>{this.#c(d,f)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=bi(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new lr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=aR(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&ue.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var Zh=class extends Mt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??yi(n,t),s=this.get(r);return s||(s=new Hh({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){ue.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>ol(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>ol(e,a)):t}notify(e){ue.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){ue.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){ue.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var gd=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new Zh,this.#e=e.mutationCache||new Jh,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=Hr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=Qr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(xa(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=Fh(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return ue.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;ue.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return ue.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=ue.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(De).catch(De)}invalidateQueries(e,t={}){return ue.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=ue.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(De)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(De)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(xa(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(De).catch(De)}fetchInfiniteQuery(e){return e.behavior=fd(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(De).catch(De)}ensureInfiniteQueryData(e){return e.behavior=fd(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return Qr.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Ta(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{or(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Ta(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{or(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=yi(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===Kr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var Aa=ze(Ke(),1);var Vr=ze(Ke(),1),av=ze(yd(),1),bd=Vr.createContext(void 0),J=e=>{let t=Vr.useContext(bd);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},xd=({client:e,children:t})=>(Vr.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,av.jsx)(bd.Provider,{value:e,children:t}));var hl=ze(Ke(),1),nv=hl.createContext(!1),vl=()=>hl.useContext(nv),mL=nv.Provider;var Si=ze(Ke(),1),sR=ze(yd(),1);function iR(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var oR=Si.createContext(iR()),gl=()=>Si.useContext(oR);var rv=ze(Ke(),1);var yl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},bl=e=>{rv.useEffect(()=>{e.clearReset()},[e])},xl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||$i(a,[e.error,n]));var $l=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},wl=(e,t)=>e.isLoading&&e.isFetching&&!t,Ni=(e,t)=>e?.suspense&&t.isPending,Gr=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function $d({queries:e,...t},a){let n=J(a),r=vl(),s=gl(),i=Aa.useMemo(()=>e.map(y=>{let $=n.defaultQueryOptions(y);return $._optimisticResults=r?"isRestoring":"optimistic",$}),[e,n,r]);i.forEach(y=>{$l(y),yl(y,s)}),bl(s);let[o]=Aa.useState(()=>new vd(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),f=!r&&t.subscribed!==!1;Aa.useSyncExternalStore(Aa.useCallback(y=>f?o.subscribe(ue.batchCalls(y)):De,[o,f]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),Aa.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let p=u.some((y,$)=>Ni(i[$],y))?u.flatMap((y,$)=>{let g=i[$];if(g){let v=new lr(n,g);if(Ni(g,y))return Gr(g,v,s);wl(y,r)&&Gr(g,v,s)}return[]}):[];if(p.length>0)throw Promise.all(p);let b=u.find((y,$)=>{let g=i[$];return g&&xl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(b?.error)throw b.error;return c(d())}var _n=ze(Ke(),1);function sv(e,t,a){let n=vl(),r=gl(),s=J(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",$l(i),yl(i,r),bl(r);let o=!s.getQueryCache().get(i.queryHash),[u]=_n.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(_n.useSyncExternalStore(_n.useCallback(f=>{let m=d?u.subscribe(ue.batchCalls(f)):De;return u.updateResult(),m},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),_n.useEffect(()=>{u.setOptions(i)},[i,u]),Ni(i,c))throw Gr(i,u,r);if(xl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ot&&wl(c,n)&&(o?Gr(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(De).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function B(e,t){return sv(e,lr,t)}var Ja=ze(Ke(),1);function H(e,t){let a=J(t),[n]=Ja.useState(()=>new hd(a,e));Ja.useEffect(()=>{n.setOptions(e)},[n,e]);let r=Ja.useSyncExternalStore(Ja.useCallback(i=>n.subscribe(ue.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=Ja.useCallback((i,o)=>{n.mutate(i,o).catch(De)},[n]);if(r.error&&$i(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var Bk=ze(R0());var ta=ze(Ke(),1),X=ze(Ke(),1),Ce=ze(Ke(),1),hp=ze(Ke(),1),J0=ze(Ke(),1),he=ze(Ke(),1),UE=ze(Ke(),1),jE=ze(Ke(),1),FE=ze(Ke(),1),Z=ze(Ke(),1),dx=ze(Ke(),1);var C0="popstate";function M0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return Wf("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:qs(r)}return j3(t,a,null,e)}function Re(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function ea(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function U3(){return Math.random().toString(36).substring(2,10)}function E0(e,t){return{usr:e.state,key:e.key,idx:t}}function Wf(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Cr(t):t,state:a,key:t&&t.key||n||U3()}}function qs({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Cr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function j3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function f(){o="POP";let $=d(),g=$==null?null:$-c;c=$,u&&u({action:o,location:y.location,delta:g})}function m($,g){o="PUSH";let v=Wf(y.location,$,g);a&&a(v,$),c=d()+1;let x=E0(v,c),w=y.createHref(v);try{i.pushState(x,"",w)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign(w)}s&&u&&u({action:o,location:y.location,delta:1})}function p($,g){o="REPLACE";let v=Wf(y.location,$,g);a&&a(v,$),c=d();let x=E0(v,c),w=y.createHref(v);i.replaceState(x,"",w),s&&u&&u({action:o,location:y.location,delta:0})}function b($){return F3($)}let y={get action(){return o},get location(){return e(r,i)},listen($){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener(C0,f),u=$,()=>{r.removeEventListener(C0,f),u=null}},createHref($){return t(r,$)},createURL:b,encodeLocation($){let g=b($);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:m,replace:p,go($){return i.go($)}};return y}function F3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Re(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:qs(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var q3;q3=new WeakMap;function np(e,t,a="/"){return z3(e,t,a,!1)}function z3(e,t,a,n){let r=typeof t=="string"?Cr(t):t,s=za(r.pathname||"/",a);if(s==null)return null;let i=O0(e);I3(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=eE(s);o=Z3(i[u],c,n)}return o}function B3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function O0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Re(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let f=hn([n,d.relativePath]),m=a.concat(d);i.children&&i.children.length>0&&(Re(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${f}".`),O0(i.children,t,m,f,u)),!(i.path==null&&!i.index)&&t.push({path:f,score:J3(f,i.index),routesMeta:m})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of L0(i.path))s(i,o,!0,u)}),t}function L0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=L0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function I3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:X3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var K3=/^:[\w-]+$/,H3=3,Q3=2,V3=1,G3=10,Y3=-2,T0=e=>e==="*";function J3(e,t){let a=e.split("/"),n=a.length;return a.some(T0)&&(n+=Y3),t&&(n+=Q3),a.filter(r=>!T0(r)).reduce((r,s)=>r+(K3.test(s)?H3:s===""?V3:G3),n)}function X3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function Z3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",f=Mo({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),m=u.route;if(!f&&c&&a&&!n[n.length-1].route.index&&(f=Mo({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!f)return null;Object.assign(r,f.params),i.push({params:r,pathname:hn([s,f.pathname]),pathnameBase:nE(hn([s,f.pathnameBase])),route:m}),f.pathnameBase!=="/"&&(s=hn([s,f.pathnameBase]))}return i}function Mo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=W3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:f},m)=>{if(d==="*"){let b=o[m]||"";i=s.slice(0,s.length-b.length).replace(/(.)\/+$/,"$1")}let p=o[m];return f&&!p?c[d]=void 0:c[d]=(p||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function W3(e,t=!1,a=!0){ea(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function eE(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return ea(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function za(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function P0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Cr(e):e;return{pathname:a?a.startsWith("/")?a:tE(a,t):t,search:rE(n),hash:sE(r)}}function tE(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function Xf(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function aE(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function rp(e){let t=aE(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function sp(e,t,a,n=!1){let r;typeof e=="string"?r=Cr(e):(r={...e},Re(!r.pathname||!r.pathname.includes("?"),Xf("?","pathname","search",r)),Re(!r.pathname||!r.pathname.includes("#"),Xf("#","pathname","hash",r)),Re(!r.search||!r.search.includes("#"),Xf("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let f=t.length-1;if(!n&&i.startsWith("..")){let m=i.split("/");for(;m[0]==="..";)m.shift(),f-=1;r.pathname=m.join("/")}o=f>=0?t[f]:"/"}let u=P0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var hn=e=>e.join("/").replace(/\/\/+/g,"/"),nE=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),rE=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,sE=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function U0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var j0=["POST","PUT","PATCH","DELETE"],XL=new Set(j0),iE=["GET",...j0],ZL=new Set(iE);var WL=Symbol("ResetLoaderData");var Er=ta.createContext(null);Er.displayName="DataRouter";var zs=ta.createContext(null);zs.displayName="DataRouterState";var e6=ta.createContext(!1);var ip=ta.createContext({isTransitioning:!1});ip.displayName="ViewTransition";var F0=ta.createContext(new Map);F0.displayName="Fetchers";var oE=ta.createContext(null);oE.displayName="Await";var zt=ta.createContext(null);zt.displayName="Navigation";var Bs=ta.createContext(null);Bs.displayName="Location";var aa=ta.createContext({outlet:null,matches:[],isDataRoute:!1});aa.displayName="Route";var op=ta.createContext(null);op.displayName="RouteError";var ep=!0;function q0(e,{relative:t}={}){Re(Tr(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=X.useContext(zt),{hash:r,pathname:s,search:i}=Is(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:hn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Tr(){return X.useContext(Bs)!=null}function Pe(){return Re(Tr(),"useLocation() may be used only in the context of a <Router> component."),X.useContext(Bs).location}var z0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function B0(e){X.useContext(zt).static||X.useLayoutEffect(e)}function de(){let{isDataRoute:e}=X.useContext(aa);return e?gE():lE()}function lE(){Re(Tr(),"useNavigate() may be used only in the context of a <Router> component.");let e=X.useContext(Er),{basename:t,navigator:a}=X.useContext(zt),{matches:n}=X.useContext(aa),{pathname:r}=Pe(),s=JSON.stringify(rp(n)),i=X.useRef(!1);return B0(()=>{i.current=!0}),X.useCallback((u,c={})=>{if(ea(i.current,z0),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=sp(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:hn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var I0=X.createContext(null);function Ba(){return X.useContext(I0)}function K0(e){let t=X.useContext(aa).outlet;return t&&X.createElement(I0.Provider,{value:e},t)}function st(){let{matches:e}=X.useContext(aa),t=e[e.length-1];return t?t.params:{}}function Is(e,{relative:t}={}){let{matches:a}=X.useContext(aa),{pathname:n}=Pe(),r=JSON.stringify(rp(a));return X.useMemo(()=>sp(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function H0(e,t){return Q0(e,t)}function Q0(e,t,a,n,r){Re(Tr(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=X.useContext(zt),{matches:i}=X.useContext(aa),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",f=o&&o.route;if(ep){let v=f&&f.path||"";Y0(c,!f||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let m=Pe(),p;if(t){let v=typeof t=="string"?Cr(t):t;Re(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),p=v}else p=m;let b=p.pathname||"/",y=b;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+b.replace(/^\//,"").split("/").slice(v.length).join("/")}let $=np(e,{pathname:y});ep&&(ea(f||$!=null,`No routes matched location "${p.pathname}${p.search}${p.hash}" `),ea($==null||$[$.length-1].route.element!==void 0||$[$.length-1].route.Component!==void 0||$[$.length-1].route.lazy!==void 0,`Matched leaf route at location "${p.pathname}${p.search}${p.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=fE($&&$.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:hn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:hn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?X.createElement(Bs.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...p},navigationType:"POP"}},g):g}function uE(){let e=G0(),t=U0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return ep&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=X.createElement(X.Fragment,null,X.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),X.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",X.createElement("code",{style:s},"ErrorBoundary")," or"," ",X.createElement("code",{style:s},"errorElement")," prop on your route."))),X.createElement(X.Fragment,null,X.createElement("h2",null,"Unexpected Application Error!"),X.createElement("h3",{style:{fontStyle:"italic"}},t),a?X.createElement("pre",{style:r},a):null,i)}var cE=X.createElement(uE,null),dE=class extends X.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?X.createElement(aa.Provider,{value:this.props.routeContext},X.createElement(op.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function mE({routeContext:e,match:t,children:a}){let n=X.useContext(Er);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),X.createElement(aa.Provider,{value:e},a)}function fE(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Re(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:f,errors:m}=a,p=d.route.loader&&!f.hasOwnProperty(d.route.id)&&(!m||m[d.route.id]===void 0);if(d.route.lazy||p){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,f)=>{let m,p=!1,b=null,y=null;a&&(m=i&&d.route.id?i[d.route.id]:void 0,b=d.route.errorElement||cE,o&&(u<0&&f===0?(Y0("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),p=!0,y=null):u===f&&(p=!0,y=d.route.hydrateFallbackElement||null)));let $=t.concat(s.slice(0,f+1)),g=()=>{let v;return m?v=b:p?v=y:d.route.Component?v=X.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,X.createElement(mE,{match:d,routeContext:{outlet:c,matches:$,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||f===0)?X.createElement(dE,{location:a.location,revalidation:a.revalidation,component:b,error:m,children:g(),routeContext:{outlet:null,matches:$,isDataRoute:!0},unstable_onError:n}):g()},null)}function lp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function pE(e){let t=X.useContext(Er);return Re(t,lp(e)),t}function up(e){let t=X.useContext(zs);return Re(t,lp(e)),t}function hE(e){let t=X.useContext(aa);return Re(t,lp(e)),t}function cp(e){let t=hE(e),a=t.matches[t.matches.length-1];return Re(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function vE(){return cp("useRouteId")}function V0(){return up("useNavigation").navigation}function dp(){let{matches:e,loaderData:t}=up("useMatches");return X.useMemo(()=>e.map(a=>B3(a,t)),[e,t])}function G0(){let e=X.useContext(op),t=up("useRouteError"),a=cp("useRouteError");return e!==void 0?e:t.errors?.[a]}function gE(){let{router:e}=pE("useNavigate"),t=cp("useNavigate"),a=X.useRef(!1);return B0(()=>{a.current=!0}),X.useCallback(async(r,s={})=>{ea(a.current,z0),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var A0={};function Y0(e,t,a){!t&&!A0[e]&&(A0[e]=!0,ea(!1,a))}var t6=Ce.memo(yE);function yE({routes:e,future:t,state:a,unstable_onError:n}){return Q0(e,void 0,a,n,t)}function it({to:e,replace:t,state:a,relative:n}){Re(Tr(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Ce.useContext(zt);ea(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Ce.useContext(aa),{pathname:i}=Pe(),o=de(),u=sp(e,rp(s),i,n==="path"),c=JSON.stringify(u);return Ce.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function mp(e){return K0(e.context)}function ve(e){Re(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function fp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Re(!Tr(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Ce.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Cr(a));let{pathname:u="/",search:c="",hash:d="",state:f=null,key:m="default"}=a,p=Ce.useMemo(()=>{let b=za(u,i);return b==null?null:{location:{pathname:b,search:c,hash:d,state:f,key:m},navigationType:n}},[i,u,c,d,f,m,n]);return ea(p!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),p==null?null:Ce.createElement(zt.Provider,{value:o},Ce.createElement(Bs.Provider,{children:t,value:p}))}function pp({children:e,location:t}){return H0(Wu(e),t)}function Wu(e,t=[]){let a=[];return Ce.Children.forEach(e,(n,r)=>{if(!Ce.isValidElement(n))return;let s=[...t,r];if(n.type===Ce.Fragment){a.push.apply(a,Wu(n.props.children,s));return}Re(n.type===ve,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Re(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=Wu(n.props.children,s)),a.push(i)}),a}var Xu="get",Zu="application/x-www-form-urlencoded";function ec(e){return e!=null&&typeof e.tagName=="string"}function bE(e){return ec(e)&&e.tagName.toLowerCase()==="button"}function xE(e){return ec(e)&&e.tagName.toLowerCase()==="form"}function $E(e){return ec(e)&&e.tagName.toLowerCase()==="input"}function wE(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function SE(e,t){return e.button===0&&(!t||t==="_self")&&!wE(e)}var Yu=null;function NE(){if(Yu===null)try{new FormData(document.createElement("form"),0),Yu=!1}catch{Yu=!0}return Yu}var _E=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function Zf(e){return e!=null&&!_E.has(e)?(ea(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${Zu}"`),null):e}function kE(e,t){let a,n,r,s,i;if(xE(e)){let o=e.getAttribute("action");n=o?za(o,t):null,a=e.getAttribute("method")||Xu,r=Zf(e.getAttribute("enctype"))||Zu,s=new FormData(e)}else if(bE(e)||$E(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?za(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||Xu,r=Zf(e.getAttribute("formenctype"))||Zf(o.getAttribute("enctype"))||Zu,s=new FormData(o,e),!NE()){let{name:c,type:d,value:f}=e;if(d==="image"){let m=c?`${c}.`:"";s.append(`${m}x`,"0"),s.append(`${m}y`,"0")}else c&&s.append(c,f)}}else{if(ec(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=Xu,n=null,r=Zu,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var a6=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function vp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var RE=Symbol("SingleFetchRedirect");function CE(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&za(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function EE(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function TE(e){return e!=null&&typeof e.page=="string"}function AE(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function DE(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await EE(s,a);return i.links?i.links():[]}return[]}));return PE(n.flat(1).filter(AE).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function D0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let f=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof f=="boolean")return f}return!0}):[]}function ME(e,t,{includeHydrateFallback:a}={}){return OE(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function OE(e){return[...new Set(e)]}function LE(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function PE(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!TE(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(LE(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function X0(){let e=he.useContext(Er);return vp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function qE(){let e=he.useContext(zs);return vp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var Oo=he.createContext(void 0);Oo.displayName="FrameworkContext";function Z0(){let e=he.useContext(Oo);return vp(e,"You must render this element inside a <HydratedRouter> element"),e}function zE(e,t){let a=he.useContext(Oo),[n,r]=he.useState(!1),[s,i]=he.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:f}=t,m=he.useRef(null);he.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},$=new IntersectionObserver(y,{threshold:.5});return m.current&&$.observe(m.current),()=>{$.disconnect()}}},[e]),he.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let p=()=>{r(!0)},b=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,m,{}]:[s,m,{onFocus:Do(o,p),onBlur:Do(u,b),onMouseEnter:Do(c,p),onMouseLeave:Do(d,b),onTouchStart:Do(f,p)}]:[!1,m,{}]}function Do(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function W0({page:e,...t}){let{router:a}=X0(),n=he.useMemo(()=>np(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?he.createElement(IE,{page:e,matches:n,...t}):null}function BE(e){let{manifest:t,routeModules:a}=Z0(),[n,r]=he.useState([]);return he.useEffect(()=>{let s=!1;return DE(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function IE({page:e,matches:t,...a}){let n=Pe(),{manifest:r,routeModules:s}=Z0(),{basename:i}=X0(),{loaderData:o,matches:u}=qE(),c=he.useMemo(()=>D0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=he.useMemo(()=>D0(e,t,u,r,n,"assets"),[e,t,u,r,n]),f=he.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let b=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(x=>x.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:b.add(g.route.id))}),b.size===0)return[];let $=CE(e,i,"data");return y&&b.size>0&&$.searchParams.set("_routes",t.filter(g=>b.has(g.route.id)).map(g=>g.route.id).join(",")),[$.pathname+$.search]},[i,o,n,r,c,t,e,s]),m=he.useMemo(()=>ME(d,r),[d,r]),p=BE(d);return he.createElement(he.Fragment,null,f.map(b=>he.createElement("link",{key:b,rel:"prefetch",as:"fetch",href:b,...a})),m.map(b=>he.createElement("link",{key:b,rel:"modulepreload",href:b,...a})),p.map(({key:b,link:y})=>he.createElement("link",{key:b,nonce:a.nonce,...y})))}function KE(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var ex=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{ex&&(window.__reactRouterVersion="7.9.1")}catch{}function gp({basename:e,children:t,window:a}){let n=Z.useRef();n.current==null&&(n.current=M0({window:a,v5Compat:!0}));let r=n.current,[s,i]=Z.useState({action:r.action,location:r.location}),o=Z.useCallback(u=>{Z.startTransition(()=>i(u))},[i]);return Z.useLayoutEffect(()=>r.listen(o),[r,o]),Z.createElement(fp,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function tx({basename:e,children:t,history:a}){let[n,r]=Z.useState({action:a.action,location:a.location}),s=Z.useCallback(i=>{Z.startTransition(()=>r(i))},[r]);return Z.useLayoutEffect(()=>a.listen(s),[a,s]),Z.createElement(fp,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}tx.displayName="unstable_HistoryRouter";var ax=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,Ar=Z.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:f,...m},p){let{basename:b}=Z.useContext(zt),y=typeof c=="string"&&ax.test(c),$,g=!1;if(typeof c=="string"&&y&&($=c,ex))try{let O=new URL(window.location.href),L=c.startsWith("//")?new URL(O.protocol+c):new URL(c),U=za(L.pathname,b);L.origin===O.origin&&U!=null?c=U+L.search+L.hash:g=!0}catch{ea(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=q0(c,{relative:r}),[x,w,S]=zE(n,m),k=ix(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:f});function N(O){t&&t(O),O.defaultPrevented||k(O)}let E=Z.createElement("a",{...m,...S,href:$||v,onClick:g||s?t:N,ref:KE(p,w),target:u,"data-discover":!y&&a==="render"?"true":void 0});return x&&!y?Z.createElement(Z.Fragment,null,E,Z.createElement(W0,{page:v})):E});Ar.displayName="Link";var Ia=Z.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let f=Is(i,{relative:c.relative}),m=Pe(),p=Z.useContext(zs),{navigator:b,basename:y}=Z.useContext(zt),$=p!=null&&cx(f)&&o===!0,g=b.encodeLocation?b.encodeLocation(f).pathname:f.pathname,v=m.pathname,x=p&&p.navigation&&p.navigation.location?p.navigation.location.pathname:null;a||(v=v.toLowerCase(),x=x?x.toLowerCase():null,g=g.toLowerCase()),x&&y&&(x=za(x,y)||x);let w=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt(w)==="/",k=x!=null&&(x===g||!r&&x.startsWith(g)&&x.charAt(g.length)==="/"),N={isActive:S,isPending:k,isTransitioning:$},E=S?t:void 0,O;typeof n=="function"?O=n(N):O=[n,S?"active":null,k?"pending":null,$?"transitioning":null].filter(Boolean).join(" ");let L=typeof s=="function"?s(N):s;return Z.createElement(Ar,{...c,"aria-current":E,className:O,ref:d,style:L,to:i,viewTransition:o},typeof u=="function"?u(N):u)});Ia.displayName="NavLink";var nx=Z.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=Xu,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:f,...m},p)=>{let b=ox(),y=lx(o,{relative:c}),$=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&ax.test(o);return Z.createElement("form",{ref:p,method:$,action:y,onSubmit:n?u:x=>{if(u&&u(x),x.defaultPrevented)return;x.preventDefault();let w=x.nativeEvent.submitter,S=w?.getAttribute("formmethod")||i;b(w||x.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:f})},...m,"data-discover":!g&&e==="render"?"true":void 0})});nx.displayName="Form";function rx({getKey:e,storageKey:t,...a}){let n=Z.useContext(Oo),{basename:r}=Z.useContext(zt),s=Pe(),i=dp();ux({getKey:e,storageKey:t});let o=Z.useMemo(()=>{if(!n||!e)return null;let c=ap(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let f=Math.random().toString(32).slice(2);window.history.replaceState({key:f},"")}try{let m=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof m=="number"&&window.scrollTo(0,m)}catch(f){console.error(f),sessionStorage.removeItem(c)}}).toString();return Z.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||tp)}, ${JSON.stringify(o)})`}})}rx.displayName="ScrollRestoration";function sx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function yp(e){let t=Z.useContext(Er);return Re(t,sx(e)),t}function HE(e){let t=Z.useContext(zs);return Re(t,sx(e)),t}function ix(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=de(),u=Pe(),c=Is(e,{relative:s});return Z.useCallback(d=>{if(SE(d,t)){d.preventDefault();let f=a!==void 0?a:qs(u)===qs(c);o(e,{replace:f,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var QE=0,VE=()=>`__${String(++QE)}__`;function ox(){let{router:e}=yp("useSubmit"),{basename:t}=Z.useContext(zt),a=vE();return Z.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=kE(n,t);if(r.navigate===!1){let d=r.fetcherKey||VE();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function lx(e,{relative:t}={}){let{basename:a}=Z.useContext(zt),n=Z.useContext(aa);Re(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Is(e||".",{relative:t})},i=Pe();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(f=>f).forEach(f=>o.append("index",f));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:hn([a,s.pathname])),qs(s)}var tp="react-router-scroll-positions",Ju={};function ap(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:za(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function ux({getKey:e,storageKey:t}={}){let{router:a}=yp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=HE("useScrollRestoration"),{basename:s}=Z.useContext(zt),i=Pe(),o=dp(),u=V0();Z.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),GE(Z.useCallback(()=>{if(u.state==="idle"){let c=ap(i,o,s,e);Ju[c]=window.scrollY}try{sessionStorage.setItem(t||tp,JSON.stringify(Ju))}catch(c){ea(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(Z.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||tp);c&&(Ju=JSON.parse(c))}catch{}},[t]),Z.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(Ju,()=>window.scrollY,e?(d,f)=>ap(d,f,s,e):void 0);return()=>c&&c()},[a,s,e]),Z.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{ea(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function GE(e,t){let{capture:a}=t||{};Z.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function cx(e,{relative:t}={}){let a=Z.useContext(ip);Re(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=yp("useViewTransitionState"),r=Is(e,{relative:t});if(!a.isTransitioning)return!1;let s=za(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=za(a.nextLocation.pathname,n)||a.nextLocation.pathname;return Mo(r.pathname,i)!=null||Mo(r.pathname,s)!=null}var Ct=new gd({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var bp="ironclaw_token",Ve="/api/webchat/v2",Dr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function ya(){return sessionStorage.getItem(bp)||""}function Ks(e){e?sessionStorage.setItem(bp,e):sessionStorage.removeItem(bp)}function tc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function fx(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function mx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function px({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=mx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=mx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function V(e,t={}){let a=ya(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await fx(r);throw new Dr(px({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function ac(){return V(`${Ve}/session`)}function nc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||tc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),V(`${Ve}/threads`,{method:"POST",body:JSON.stringify(n)})}function hx({limit:e,cursor:t}={}){let a=new URL(`${Ve}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),V(a.pathname+a.search)}function vx({threadId:e}={}){return e?V(`${Ve}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function xp(e){return`${Ve}/threads/${encodeURIComponent(e)}/files`}function gx({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(xp(e),window.location.origin);return t&&a.searchParams.set("path",t),V(a.pathname+a.search)}function yx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${xp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),V(a.pathname+a.search)}function rc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${xp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function bx({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return V(`${Ve}/automations${r?`?${r}`:""}`)}function xx({automationId:e}={}){return e?V(`${Ve}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function $x({automationId:e}={}){return e?V(`${Ve}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function wx({automationId:e}={}){return e?V(`${Ve}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var Sx=`${Ve}/projects`;function YE(e){return`${Sx}/${encodeURIComponent(e)}`}function Nx({limit:e}={}){let t=new URL(Sx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),V(t.pathname+t.search)}function _x({projectId:e}={}){return e?V(YE(e)):Promise.reject(new Error("projectId is required"))}function kx(){return V(`${Ve}/outbound/preferences`)}function Rx(){return V(`${Ve}/outbound/targets`)}function Cx({finalReplyTargetId:e}={}){return V(`${Ve}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Ex({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:f}={}){let m=new URL(`${Ve}/operator/logs`,window.location.origin);return e!=null&&m.searchParams.set("limit",String(e)),t&&m.searchParams.set("cursor",t),a&&m.searchParams.set("level",a),n&&m.searchParams.set("target",n),r&&m.searchParams.set("thread_id",r),s&&m.searchParams.set("run_id",s),i&&m.searchParams.set("turn_id",i),o&&m.searchParams.set("tool_call_id",o),u&&m.searchParams.set("tool_name",u),c&&m.searchParams.set("source",c),d&&m.searchParams.set("tail","true"),f&&m.searchParams.set("follow","true"),V(m.pathname+m.search)}function Tx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||tc(),content:t};return a.length>0&&(r.attachments=a),V(`${Ve}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function Ax({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ve}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),V(n.pathname+n.search)}function Dx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ve}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Na(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new Dr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=ya(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await fx(r);throw new Dr(px({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function $p(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function sc(e){return $p(await Na(e))}function Mx({threadId:e,afterCursor:t}={}){let a=new URL(`${Ve}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=ya();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Ox({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||tc()};return a&&(r.reason=a),V(`${Ve}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function wp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||tc(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),V(`${Ve}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function Lx({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return V("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function Px(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),V(`${Ve}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function Hs(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function Ux(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function jx(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new Dr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new Dr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function Fx(){let e=ya();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var ic="anon",qx=ic;function zx(e){qx=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:ic}function $t(){return qx}var Bx="ironclaw:v2-thread-pins:",Sp=new Set,vn=new Set,Np=null;function _p(){return`${Bx}${$t()}`}function JE(){try{let e=window.localStorage.getItem(_p());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function XE(){try{vn.size===0?window.localStorage.removeItem(_p()):window.localStorage.setItem(_p(),JSON.stringify([...vn]))}catch{}}function Ix(){let e=$t();if(e!==Np){vn.clear();for(let t of JE())vn.add(t);Np=e}}function Kx(){return new Set(vn)}function Hx(){let e=Kx();for(let t of Sp)try{t(e)}catch{}}function Qx(e){e&&(Ix(),vn.has(e)?vn.delete(e):vn.add(e),XE(),Hx())}function Vx(){return Ix(),Kx()}function Gx(e){return Sp.add(e),()=>{Sp.delete(e)}}function Yx(){vn.clear(),Np=$t();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(Bx)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}Hx()}var ZE=0,Mr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function kp(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function Jx(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":WE(t)?"text":"download"}function WE(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Lo(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function eT(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function tT(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function aT(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function Xx(e,{limits:t,existing:a=[],t:n}){let r=t||Mr,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!eT(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Lo(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Lo(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await tT(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:f,base64:m}=aT(d,c.type),p=f||"application/octet-stream",b=kp(p);s.push({id:`staged-${ZE++}`,filename:c.name||"attachment",mimeType:p,kind:b,sizeBytes:c.size,sizeLabel:Lo(c.size),dataBase64:m,previewUrl:b==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function Zx(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function Wx(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function nT(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||kp(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Dx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Lo(n.size_bytes):"",preview_url:null,fetch_url:s}})}function t$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=oT(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:e$(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=iT(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:nT(s,a),timestamp:e$(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:sT(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=rT(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function rT(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function sT(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function iT(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function e$(e){return e.received_at||e.created_at||null}function oT(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:Rp(t)}var lT="gate_declined";function Rp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=r$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Uo(e.title||e.capability_id)||"tool",toolStatus:n$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(a$(a)||e.output_summary||e.output_preview||e.result_ref)||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Cp(e){let t=r$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Uo(e.capability_id)||"tool",toolStatus:n$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:a$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function a$(e){return e||null}function Po(e){return e==="success"||e==="error"||e==="declined"}function Uo(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function n$(e,t=null){if(t===lT)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function r$(e){let t=Number(e);return Number.isFinite(t)?t:null}var uT=50,Ka=new Map,cT=30;function oc(e,t){for(Ka.delete(e),Ka.set(e,t);Ka.size>cT;){let a=Ka.keys().next().value;Ka.delete(a)}}function jo(e){return`${$t()}:${e}`}function i$(){Ka.clear()}function o$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Ka.get(jo(e)):null,[s,i]=h.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=h.default.useRef(new Set),u=h.default.useRef(e);u.current=e;let c=h.default.useCallback(async(f,m={})=>{let{preserveClientOnly:p=!1,finalReplyTimestampByRun:b=null}=m;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=$t(),$=jo(e);i(g=>({...g,isLoading:!0}));try{let g=await Ax({threadId:e,limit:uT,cursor:f});if($t()!==y)return;let v=f?[]:a?.()||[],x=t$(g.messages||[],v,e),w=g.next_cursor||null;if(f||n?.([]),!f){let S=Ka.get($)?.messages||[],k=s$(x,S,{preserveClientOnly:p,finalReplyTimestampByRun:b});oc($,{messages:k,nextCursor:w})}i(S=>{if(u.current!==e)return S;let k;return f?k=dT(x,S.messages):k=s$(x,S.messages,{preserveClientOnly:p,finalReplyTimestampByRun:b}),oc($,{messages:k,nextCursor:w}),{messages:k,nextCursor:w,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),$t()!==y)return;i(v=>u.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);h.default.useEffect(()=>{let f=e?Ka.get(jo(e)):null;i({messages:f?.messages||[],nextCursor:f?.nextCursor||null,isLoading:!!e&&!f,loadError:null}),e&&c()},[e,c]);let d=h.default.useCallback((f,m)=>{if(!f)return;let p=jo(f),b=Ka.get(p)||{messages:[],nextCursor:null},y=typeof m=="function"?m(b.messages||[]):m;oc(p,{messages:y,nextCursor:b.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:f=>i(m=>{let p=typeof f=="function"?f(m.messages):f;return e&&oc(jo(e),{messages:p,nextCursor:m.nextCursor}),{...m,messages:p}})}}function dT(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function s$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=fT(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(u=>u?.id).filter(Boolean)),o=t.filter(u=>!u||typeof u.id!="string"||i.has(u.id)?!1:pT(u)?!0:typeof u.timelineMessageId=="string"&&i.has(`msg-${u.timelineMessageId}`)?!1:mT(u)?!0:n&&u.id.startsWith("err-"));return o.length>0?[...s,...o]:s}function mT(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function fT(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])!i||!i.timestamp||(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Ep(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||i.timestamp||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,u=r.get(i.id)||(Ep(i)&&o?s.get(o):null),c=Ep(i)&&o?n?.[o]:null,d=u?.timestamp||c;return d?{...i,timestamp:d}:i})}function Ep(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function pT(e){return e?.role==="tool_activity"||e?.role==="thinking"}var qo="__new__",l$="ironclaw:v2-draft:";function Qs(e){return`${l$}${$t()}:${e||qo}`}function Tp(e){try{return window.localStorage.getItem(Qs(e))||""}catch{return""}}function Ap(e,t){try{t?window.localStorage.setItem(Qs(e),t):window.localStorage.removeItem(Qs(e))}catch{}}function u$(e){Ap(e,"")}var Fo=new Map;function Dp(e){return Fo.get(Qs(e))||[]}function c$(e,t){let a=Qs(e);t&&t.length>0?Fo.set(a,t):Fo.delete(a)}function d$(e){Fo.delete(Qs(e))}function m$(){Fo.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(l$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function hT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function vT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function gT(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=hT(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?vT(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),ya()?"":(Ks(n),n)}function yT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var bT={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function xT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),bT[t]||"Could not complete sign-in. Please try again."):""}function f$(){let[e,t]=h.default.useState(()=>gT()||ya()),[a,n]=h.default.useState(()=>xT()),[r]=h.default.useState(()=>yT()),[s,i]=h.default.useState(null),[o,u]=h.default.useState(()=>!!(r&&!ya())),[c,d]=h.default.useState(()=>!!ya());h.default.useEffect(()=>{if(!r||ya()){u(!1);return}let b=!1;return jx(r).then(y=>{b||(Ks(y),d(!0),t(y),i(null),n(""),u(!1),Ct.clear())}).catch(()=>{b||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{b=!0}},[r]),h.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let b=!1;return d(!0),ac().then(y=>{b||(i(y),d(!1))}).catch(y=>{b||(i(null),d(!1),(y?.status===401||y?.status===403)&&(Ks(""),t(""),n("Your session expired. Please sign in again."),Ct.clear()))}),()=>{b=!0}},[e,o]),zx(s);let f=h.default.useRef(null);h.default.useEffect(()=>{let b=$t();f.current&&f.current!==ic&&f.current!==b&&(i$(),m$(),Yx()),f.current=b},[s]);let m=h.default.useCallback(b=>{Ks(b),d(!!b),t(b),i(null),n(""),Ct.clear()},[]),p=h.default.useCallback(()=>{Fx().catch(()=>{}),Ks(""),d(!1),t(""),i(null),n(""),Ct.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:m,signOut:p}}var Or="/chat",zo=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var $T=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],wT=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],ST=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],lc={settings:$T,extensions:wT,admin:ST};var p$="ironclaw:v2-theme";function NT(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(p$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function uc(){let[e,t]=h.default.useState(NT);h.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(p$,e)}catch{}},[e]);let a=h.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function h$(e){return B({enabled:!!e,queryKey:["gateway-status",e],queryFn:Hs,refetchInterval:3e4})}function v$(){return Promise.resolve({settings:{},todo:!0})}function g$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function y$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 settings endpoint"})}function cc(){return V("/api/webchat/v2/llm/providers")}function b$(e){return V("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function x$(e){return V(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function Bo(e){return V("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function $$(e){return V("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function w$(e){return V("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function S$(e){return V("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function N$(e){return V("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function _$(){return V("/api/webchat/v2/llm/codex/login",{method:"POST"})}function k$(){return Promise.resolve({tools:[],todo:!0})}function R$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 tools endpoint"})}function C$(){return V("/api/webchat/v2/extensions")}function E$(){return V("/api/webchat/v2/extensions/registry")}function T$(){return V("/api/webchat/v2/skills")}function A$(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function D$(e){return V("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function M$(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function O$(e){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function L$(e,t){return V(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function P$(e){return V("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function U$(){return V("/api/webchat/v2/traces/credit")}function j$(e){return V(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function F$(){return Promise.resolve({users:[],todo:!0})}function q$(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function z$(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Mp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Op=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function Io(e){return Op.find(t=>t.value===e)?.label||e}function Vs(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function B$(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function dc(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function I$(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function Lr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Mp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?Vs(e,t).trim().length>0:!0:!1}function _T(e,t,a){return e.id===a?"active":Lr(e,t)?"ready":"setup"}function K$(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=_T(r,t,a);n[s]&&n[s].push(r)}return n}function mc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Mp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!Vs(e,t).trim()?"base_url":"ok"}function Lp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Mp&&(i.api_key=void 0),i}function H$(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function Q$(e){return/^[a-z0-9_-]+$/.test(e)}function V$(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var kT=Object.freeze({});function Gs({settings:e,gatewayStatus:t,enabled:a=!0}){let n=J(),r=B({queryKey:["llm-providers"],queryFn:cc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=kT,u=(s.providers||[]).map(w=>({...w,name:w.description,has_api_key:w.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,f=d||"nearai",m=s.active?.model||t?.llm_model||"",p=u.filter(w=>w.builtin),b=u.filter(w=>!w.builtin),y=[...u].sort((w,S)=>w.id===d?-1:S.id===d?1:(w.name||w.id).localeCompare(S.name||S.id)),$=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=H({mutationFn:async w=>{if(!Lr(w,o)){let k=mc(w,o);throw new Error(k==="base_url"?"base_url":"api_key")}let S=dc(w,o);if(!S)throw new Error("model");return await Bo({provider_id:w.id,model:S}),w},onSuccess:$}),v=H({mutationFn:async({provider:w,form:S,apiKey:k,editingProvider:N})=>{let E=!!w?.builtin,L={id:(E?w.id:S.id.trim()).trim(),name:E?w.name||w.id:S.name.trim(),adapter:E?w.adapter:S.adapter,base_url:S.baseUrl.trim()||w?.base_url||"",default_model:S.model.trim()||void 0};return k.trim()&&(L.api_key=k.trim()),(N||w)?.id===f&&L.default_model&&(L.set_active=!0,L.model=L.default_model),await b$(L),L},onSuccess:$}),x=H({mutationFn:async w=>(await x$(w.id),w),onSuccess:$});return{providers:y,builtinProviders:p,customProviders:b,builtinOverrides:o,activeProviderId:d,selectedModel:m,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:w=>g.mutateAsync(w),saveCustomProvider:w=>v.mutateAsync(w),saveBuiltinProvider:w=>v.mutateAsync(w),deleteCustomProvider:w=>x.mutateAsync(w),testConnection:$$,listModels:w$,isBusy:g.isPending||v.isPending||x.isPending}}function G$({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var Y$="ironclaw:v2-sidebar-open";function J$(){return typeof window>"u"?null:window}function X$(){try{return J$()?.localStorage||null}catch{return null}}function Z$(e=X$()){try{return e?.getItem(Y$)!=="false"}catch{return!0}}function W$(e,t=X$()){try{t?.setItem(Y$,e?"true":"false")}catch{}}function ew(e=J$()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function tw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function aw(e,t){return t?e.desktopOpen:e.mobileOpen}function nw({onNewChat:e}={}){let t=de(),[a,n]=h.default.useState(()=>({mobileOpen:!1,desktopOpen:Z$()})),[r,s]=h.default.useState(()=>ew());h.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),f=()=>s(d.matches);return f(),d.addEventListener?.("change",f),()=>d.removeEventListener?.("change",f)},[]),h.default.useEffect(()=>{W$(a.desktopOpen)},[a.desktopOpen]);let i=h.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=h.default.useCallback(()=>{n(d=>tw(d,r))},[r]),u=h.default.useCallback(async()=>{let d=await e?.(),f=typeof d=="string"&&d.length>0?d:null;t(f?`/chat/${f}`:"/chat"),i()},[t,i,e]),c=h.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:aw(a,r),close:i,toggle:o,newChat:u,selectThread:c}}var Pp=new Set,RT=0;function Ys(e,t={}){let a={id:++RT,message:e,tone:t.tone||"info",duration:t.duration??2600};return Pp.forEach(n=>n(a)),a.id}function rw(e){return Pp.add(e),()=>Pp.delete(e)}function CT(e){return e?.status===409&&e?.payload?.kind==="busy"}function sw(e,t){return CT(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function iw(){let e=B({queryKey:["threads"],queryFn:()=>hx({})}),[t,a]=h.default.useState(null),[n,r]=h.default.useState(!1),s=h.default.useRef(new Map),i=h.default.useCallback(async c=>{let d=c||"__global__",f=s.current.get(d);if(f)return f;r(!0);let m=(async()=>{try{let p=await nc(c?{projectId:c}:void 0);Ct.invalidateQueries({queryKey:["threads"]});let b=p?.thread?.thread_id;return b&&a(b),b}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,m),m},[]),o=h.default.useCallback(async c=>{await vx({threadId:c}),t===c&&a(null),Ct.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:h.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var ow={attach:l`<path
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
      ${ow[e]||ow.spark}
    </svg>
  `}function Q(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=Q(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function lw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function ET(e){return lw(e).trim().charAt(0).toUpperCase()||"I"}function TT(){let[e,t]=h.default.useState(!1),a=h.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function uw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=R(),s=TT(),i=lw(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      ${s.open&&l`
        <div
          className=${Q("absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg","border-[var(--v2-panel-border)] bg-[var(--v2-surface)]")}
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
            />`:l`<span className="place-self-center">${ET(a)}</span>`}
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
  `}var cw={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",settings:"settings",admin:"shield"},AT=zo.filter(e=>e.id!=="chat"&&!e.hidden);function DT({route:e,label:t,onNavigate:a}){return l`
    <${Ia}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>Q("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${M} name=${cw[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function MT({route:e,label:t,subRoutes:a,onNavigate:n}){let r=R(),s=Pe(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Ia}
        to=${o}
        onClick=${n}
        className=${()=>Q("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${M}
          name=${cw[e.id]||"bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${t}</span>
        <${M}
          name="chevron"
          className=${Q("h-3.5 w-3.5 shrink-0 transition-transform duration-150",i&&"rotate-180")}
        />
      <//>

      ${i&&l`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${a.map(u=>l`
              <${Ia}
                key=${u.id}
                to=${e.path+"/"+u.id}
                onClick=${n}
                className=${({isActive:c})=>Q("flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",c?"text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
              >
                <${M} name=${u.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${r(u.labelKey)}</span>
              <//>
            `)}
        </div>
      `}
    </div>
  `}function dw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=R(),s=h.default.useMemo(()=>AT.filter(i=>a||i.id!=="admin"),[a]);return l`
    <div className="flex flex-col px-3 py-2">
      <button
        onClick=${e}
        disabled=${t}
        className=${Q("flex items-center gap-2.5 rounded-[10px] px-3 py-2","border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]","bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]","hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50")}
      >
        <${M} name="plus" className="h-4 w-4 shrink-0" />
        <span className="text-[13px] font-medium">
          ${r(t?"chat.creating":"chat.newThread")}
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${s.map(i=>{let o=(lc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${MT}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${DT}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var gn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),Ko=new Set([gn.NEEDS_ATTENTION,gn.FAILED]),Up="ironclaw:v2-thread-attention",jp=new Set,Js=new Map;function OT(){try{let e=window.localStorage.getItem(Up);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&Ko.has(a[1])):[]}catch{return[]}}function mw(){let e=[];for(let[t,a]of Js)Ko.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Up):window.localStorage.setItem(Up,JSON.stringify(e))}catch{}}for(let[e,t]of OT())Js.set(e,t);function pw(){return new Map(Js)}function fw(){let e=pw();for(let t of jp)try{t(e)}catch{}}function fc(e,t){if(!e)return;let a=Js.get(e);if(t==null){if(!Js.delete(e))return;Ko.has(a)&&mw(),fw();return}a!==t&&(Js.set(e,t),(Ko.has(t)||Ko.has(a))&&mw(),fw())}function hw(e){fc(e,null)}function LT(){return pw()}function PT(e){return jp.add(e),()=>{jp.delete(e)}}function vw(){let[e,t]=h.default.useState(LT);return h.default.useEffect(()=>PT(t),[]),e}function pc(e){return e.updated_at||e.created_at||null}function Fp(e,t){let a=pc(e)||"",n=pc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function gw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function yw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function UT(){let[e,t]=h.default.useState(Vx);return h.default.useEffect(()=>Gx(t),[]),e}var jT=Object.freeze({[gn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[gn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[gn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function FT(e){return e&&jT[e]||null}function qT({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=R(),o=pc(e),u=gw(o),c=yw(o),d=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(p=>{window.alert(p?.message||"Unable to delete chat")})},[s,e.id]),f=h.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),Qx(e.id)},[e.id]);return l`
    <div
      className=${Q("group flex w-full items-stretch rounded-[8px] border-l-2",n?n.borderClass:t?"border-[var(--v2-accent)]":"border-transparent",t?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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
            className=${Q("h-1.5 w-1.5 shrink-0 rounded-full",n.dotClass)}
          />`}
        </div>
        ${(n||u)&&l`<span
          className=${Q("block truncate text-[11px]",n?n.textClass:"text-[var(--v2-text-faint)]")}
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
        className=${Q("my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",a?"text-[var(--v2-accent-text)]":"opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100","hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]")}
      >
        <${M} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${s&&l`<button
        type="button"
        onClick=${d}
        title=${i("common.deleteChat")}
        aria-label=${i("common.deleteChat")}
        className=${Q("my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]","opacity-0 transition group-hover:opacity-100 focus:opacity-100","text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]")}
      >
        <${M} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `}function bw({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${qT}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${FT(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function xw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=h.default.useState(!1),[u,c]=h.default.useState(""),d=vw(),f=UT(),m=R(),{pinned:p,recent:b,totalMatches:y}=h.default.useMemo(()=>{let $=u.trim().toLowerCase(),g=$?e.filter(w=>(w.title||w.id||"").toLowerCase().includes($)):e,v=[],x=[];for(let w of g)f.has(w.id)?v.push(w):x.push(w);return v.sort(Fp),x.sort(Fp),{pinned:v,recent:x,totalMatches:v.length+x.length}},[e,u,f]);return l`
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
        <${M}
          name="chevron"
          className=${Q("h-3.5 w-3.5 text-[var(--v2-text-faint)]",i?"-rotate-90":"")}
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
            placeholder=${m("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${a&&l`<div className="mb-1 px-1">
          <${Ia}
            to="/projects"
            onClick=${s}
            className=${({isActive:$})=>Q("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",$?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
          >
            <${M} name="folder" className="h-4 w-4 shrink-0" />
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

          <${bw}
            label=${m("common.pinned")}
            items=${p}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${f}
            onSelect=${n}
            onDelete=${r}
          />
          <${bw}
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
  `}function hc(){let e=J(),t=B({queryKey:["trace-credits"],queryFn:U$,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=H({mutationFn:j$,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function zT(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function $w(){let e=R(),{credits:t}=hc();if(!t||!t.enrolled)return null;let a=zT(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${Ar}
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
  `}function ww({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:u,onNewChat:c,onSelectThread:d,onDeleteThread:f}){return l`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Ar}
          to="/chat"
          onClick=${u}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${dw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${u}
      />

      <${$w} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${xw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${f}
          onNavigate=${u}
        />
      </div>

      <${uw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var BT="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",IT="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",Sw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",Nw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},_w={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function T({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=Nw[n]??Nw.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:BT,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${Q(Sw,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:IT}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=_w[a]??_w.outline;return l`
    <${s}
      className=${Q(Sw,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function kw(){let e=h.default.useMemo(()=>KT(window.location),[]),[t,a]=h.default.useState(null),[n,r]=h.default.useState(null),[s,i]=h.default.useState(!1),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!1);h.default.useEffect(()=>{if(!e)return;let p=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:p.signal}).then(b=>{if(!b.ok)throw new Error(String(b.status));return b.json()}).then(a).catch(()=>{p.signal.aborted||a(null)}),()=>p.abort()},[e]);let f=h.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let p=await fetch(`${e.base}/attestation/report`);if(!p.ok)throw new Error(String(p.status));let b=await p.json();return r(b),b}catch(p){return u(p.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),m=h.default.useCallback(async()=>{let p=n||await f();return!p||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...p,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[f,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:f,copyReport:m}}function KT(e){let t=e.hostname;if(!t||t==="localhost"||HT(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function HT(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var QT=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function Rw(){let e=R(),t=kw(),[a,n]=h.default.useState(!1),r=h.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=h.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=VT({teeInfo:t.teeInfo,report:t.report,t:e});return l`
    <div className="relative">
      <button
        type="button"
        onClick=${r}
        aria-expanded=${a}
        title=${e("tee.title")}
        className=${Q("grid h-8 w-8 place-items-center rounded-[8px]","border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]","bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]","hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]")}
      >
        <${M} name="shield" className="h-4 w-4" />
      </button>

      ${a&&l`
        <div
          className=${Q("absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]","rounded-[14px] border border-[var(--v2-panel-border)]","bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]")}
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
            <${T}
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
  `}function VT({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return QT.map(([r,s])=>({label:a(s),value:GT(n[r])||a("common.unknown")}))}function GT(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var YT="https://docs.ironclaw.com";function Cw({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=R(),r=Pe(),s=h.default.useMemo(()=>{for(let o of zo){let u=lc[o.id];if(!u)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],f=u.find(m=>m.id===d);if(f)return{parent:n(o.labelKey),current:n(f.labelKey)}}}return null},[r.pathname,n]),i=h.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=zo.find(u=>r.pathname.startsWith(u.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return l`
    <header
      className=${Q("flex h-14 shrink-0 items-center gap-3 px-4","border-b border-[var(--v2-panel-border)]","bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl")}
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
        <${Rw} />
        <${Ia}
          to="/logs"
          className=${({isActive:o})=>Q("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${YT}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function Ew({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=de(),i=R(),[o,u]=h.default.useState(""),[c,d]=h.default.useState(0),f=h.default.useRef(null),m=h.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(x=>({id:`thread-${x.id}`,label:x.title||`Thread ${x.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${x.id}`)}));return[...g,...v]},[a,s,n,r]),p=h.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?m.filter(v=>v.label.toLowerCase().includes(g)):m},[m,o]);h.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>f.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),h.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,p.length-1)))},[p.length]);let b=h.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=h.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,p.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),b(p[c])):g.key==="Escape"&&(g.preventDefault(),t())},[p,c,b,t]);if(!e)return null;let $=null;return l`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${t} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${M} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
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
                  <${M} name=${g.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${g.label}</span>
                </button>
              </li>
            `})}
        </ul>
      </div>
    </div>
  `}var Tw={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},JT={info:"bolt",success:"check",error:"close"};function Aw(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>rw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",Tw[a.tone]||Tw.info].join(" ")}
          >
            <${M} name=${JT[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function Dw({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=R(),{theme:o,toggleTheme:u}=uc(),c=h$(e),d=iw(),f=nw({onNewChat:()=>d.setActiveThreadId(null)}),m=c.data,p=Pe(),b=de(),y=Gs({settings:{},gatewayStatus:m,enabled:n}),$=n&&G$({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=p.pathname==="/welcome"||p.pathname.startsWith("/settings"),[v,x]=h.default.useState(!1);h.default.useEffect(()=>{let S=k=>{(k.metaKey||k.ctrlKey)&&k.key.toLowerCase()==="k"&&(k.preventDefault(),x(N=>!N))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let w=h.default.useCallback(async S=>{let k=d.activeThreadId===S;try{await d.deleteThread(S),k&&b("/chat",{replace:!0})}catch(N){console.error("Failed to delete thread:",N),Ys(sw(N,i),{tone:"error"})}},[b,d,i]);return $&&!g?l`<${it} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${f.mobileOpen&&l`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${f.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${Q("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",f.mobileOpen?"flex":"hidden",f.desktopOpen?"md:flex":"md:hidden")}
      >
        <${ww}
          id="gateway-sidebar"
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
        <${Cw}
          threadsState=${d}
          onToggleSidebar=${f.toggle}
          sidebarOpen=${f.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&l`
            <div
              className=${Q("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${mp}
            context=${{gatewayStatus:m,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${Ew}
        open=${v}
        onClose=${()=>x(!1)}
        threadsState=${d}
        onNewChat=${f.newChat}
        onToggleTheme=${u}
      />
      <${Aw} />
    </div>
  `}var Bt=ze(Ke(),1),Yo=e=>e.type==="checkbox",Pr=e=>e instanceof Date,Et=e=>e==null,Hw=e=>typeof e=="object",Ge=e=>!Et(e)&&!Array.isArray(e)&&Hw(e)&&!Pr(e),XT=e=>Ge(e)&&e.target?Yo(e.target)?e.target.checked:e.target.value:e,ZT=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,WT=(e,t)=>e.has(ZT(t)),eA=e=>{let t=e.constructor&&e.constructor.prototype;return Ge(t)&&t.hasOwnProperty("isPrototypeOf")},Bp=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function mt(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(Bp&&(e instanceof Blob||n))&&(a||Ge(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!eA(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=mt(e[r]));else return e;return t}var xc=e=>/^\w*$/.test(e),We=e=>e===void 0,Ip=e=>Array.isArray(e)?e.filter(Boolean):[],Kp=e=>Ip(e.replace(/["|']|\]/g,"").split(/\.|\[/)),Y=(e,t,a)=>{if(!t||!Ge(e))return a;let n=(xc(t)?[t]:Kp(t)).reduce((r,s)=>Et(r)?r:r[s],e);return We(n)||n===e?We(e[t])?a:e[t]:n},Ha=e=>typeof e=="boolean",Ue=(e,t,a)=>{let n=-1,r=xc(t)?[t]:Kp(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Ge(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},Mw={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},_a={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},yn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},tA=Bt.default.createContext(null);tA.displayName="HookFormContext";var aA=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==_a.all&&(t._proxyFormState[i]=!n||_a.all),a&&(a[i]=!0),e[i]}});return r},nA=typeof window<"u"?Bt.default.useLayoutEffect:Bt.default.useEffect;var Qa=e=>typeof e=="string",rA=(e,t,a,n,r)=>Qa(e)?(n&&t.watch.add(e),Y(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),Y(a,s))):(n&&(t.watchAll=!0),a),zp=e=>Et(e)||!Hw(e);function Wn(e,t,a=new WeakSet){if(zp(e)||zp(t))return e===t;if(Pr(e)&&Pr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(Pr(i)&&Pr(o)||Ge(i)&&Ge(o)||Array.isArray(i)&&Array.isArray(o)?!Wn(i,o,a):i!==o)return!1}}return!0}var sA=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},Vo=e=>Array.isArray(e)?e:[e],Ow=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},It=e=>Ge(e)&&!Object.keys(e).length,Hp=e=>e.type==="file",ka=e=>typeof e=="function",gc=e=>{if(!Bp)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},Qw=e=>e.type==="select-multiple",Qp=e=>e.type==="radio",iA=e=>Qp(e)||Yo(e),qp=e=>gc(e)&&e.isConnected;function oA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=We(e)?n++:e[t[n++]];return e}function lA(e){for(let t in e)if(e.hasOwnProperty(t)&&!We(e[t]))return!1;return!0}function Ze(e,t){let a=Array.isArray(t)?t:xc(t)?[t]:Kp(t),n=a.length===1?e:oA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ge(n)&&It(n)||Array.isArray(n)&&lA(n))&&Ze(e,a.slice(0,-1)),e}var Vw=e=>{for(let t in e)if(ka(e[t]))return!0;return!1};function yc(e,t={}){let a=Array.isArray(e);if(Ge(e)||a)for(let n in e)Array.isArray(e[n])||Ge(e[n])&&!Vw(e[n])?(t[n]=Array.isArray(e[n])?[]:{},yc(e[n],t[n])):Et(e[n])||(t[n]=!0);return t}function Gw(e,t,a){let n=Array.isArray(e);if(Ge(e)||n)for(let r in e)Array.isArray(e[r])||Ge(e[r])&&!Vw(e[r])?We(t)||zp(a[r])?a[r]=Array.isArray(e[r])?yc(e[r],[]):{...yc(e[r])}:Gw(e[r],Et(t)?{}:t[r],a[r]):a[r]=!Wn(e[r],t[r]);return a}var Ho=(e,t)=>Gw(e,t,yc(t)),Lw={value:!1,isValid:!1},Pw={value:!0,isValid:!0},Yw=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!We(e[0].attributes.value)?We(e[0].value)||e[0].value===""?Pw:{value:e[0].value,isValid:!0}:Pw:Lw}return Lw},Jw=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>We(e)?e:t?e===""?NaN:e&&+e:a&&Qa(e)?new Date(e):n?n(e):e,Uw={isValid:!1,value:null},Xw=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,Uw):Uw;function jw(e){let t=e.ref;return Hp(t)?t.files:Qp(t)?Xw(e.refs).value:Qw(t)?[...t.selectedOptions].map(({value:a})=>a):Yo(t)?Yw(e.refs).value:Jw(We(t.value)?e.ref.value:t.value,e)}var uA=(e,t,a,n)=>{let r={};for(let s of e){let i=Y(t,s);i&&Ue(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},bc=e=>e instanceof RegExp,Qo=e=>We(e)?e:bc(e)?e.source:Ge(e)?bc(e.value)?e.value.source:e.value:e,Fw=e=>({isOnSubmit:!e||e===_a.onSubmit,isOnBlur:e===_a.onBlur,isOnChange:e===_a.onChange,isOnAll:e===_a.all,isOnTouch:e===_a.onTouched}),qw="AsyncFunction",cA=e=>!!e&&!!e.validate&&!!(ka(e.validate)&&e.validate.constructor.name===qw||Ge(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===qw)),dA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),zw=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),Go=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=Y(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(Go(o,t))break}else if(Ge(o)&&Go(o,t))break}}};function Bw(e,t,a){let n=Y(e,a);if(n||xc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=Y(t,s),o=Y(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var mA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return It(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||_a.all))},fA=(e,t,a)=>!e||!t||e===t||Vo(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),pA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,hA=(e,t)=>!Ip(Y(e,t)).length&&Ze(e,t),vA=(e,t,a)=>{let n=Vo(Y(e,a));return Ue(n,"root",t[a]),Ue(e,a,n),e},vc=e=>Qa(e);function Iw(e,t,a="validate"){if(vc(e)||Array.isArray(e)&&e.every(vc)||Ha(e)&&!e)return{type:a,message:vc(e)?e:"",ref:t}}var Xs=e=>Ge(e)&&!bc(e)?e:{value:e,message:""},Kw=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:f,max:m,pattern:p,validate:b,name:y,valueAsNumber:$,mount:g}=e._f,v=Y(a,y);if(!g||t.has(y))return{};let x=o?o[0]:i,w=D=>{r&&x.reportValidity&&(x.setCustomValidity(Ha(D)?"":D||""),x.reportValidity())},S={},k=Qp(i),N=Yo(i),E=k||N,O=($||Hp(i))&&We(i.value)&&We(v)||gc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,L=sA.bind(null,y,n,S),U=(D,K,te,re=yn.maxLength,Ae=yn.minLength)=>{let Fe=D?K:te;S[y]={type:D?re:Ae,message:Fe,ref:i,...L(D?re:Ae,Fe)}};if(s?!Array.isArray(v)||!v.length:u&&(!E&&(O||Et(v))||Ha(v)&&!v||N&&!Yw(o).isValid||k&&!Xw(o).isValid)){let{value:D,message:K}=vc(u)?{value:!!u,message:u}:Xs(u);if(D&&(S[y]={type:yn.required,message:K,ref:x,...L(yn.required,K)},!n))return w(K),S}if(!O&&(!Et(f)||!Et(m))){let D,K,te=Xs(m),re=Xs(f);if(!Et(v)&&!isNaN(v)){let Ae=i.valueAsNumber||v&&+v;Et(te.value)||(D=Ae>te.value),Et(re.value)||(K=Ae<re.value)}else{let Ae=i.valueAsDate||new Date(v),Fe=Kt=>new Date(new Date().toDateString()+" "+Kt),St=i.type=="time",ot=i.type=="week";Qa(te.value)&&v&&(D=St?Fe(v)>Fe(te.value):ot?v>te.value:Ae>new Date(te.value)),Qa(re.value)&&v&&(K=St?Fe(v)<Fe(re.value):ot?v<re.value:Ae<new Date(re.value))}if((D||K)&&(U(!!D,te.message,re.message,yn.max,yn.min),!n))return w(S[y].message),S}if((c||d)&&!O&&(Qa(v)||s&&Array.isArray(v))){let D=Xs(c),K=Xs(d),te=!Et(D.value)&&v.length>+D.value,re=!Et(K.value)&&v.length<+K.value;if((te||re)&&(U(te,D.message,K.message),!n))return w(S[y].message),S}if(p&&!O&&Qa(v)){let{value:D,message:K}=Xs(p);if(bc(D)&&!v.match(D)&&(S[y]={type:yn.pattern,message:K,ref:i,...L(yn.pattern,K)},!n))return w(K),S}if(b){if(ka(b)){let D=await b(v,a),K=Iw(D,x);if(K&&(S[y]={...K,...L(yn.validate,K.message)},!n))return w(K.message),S}else if(Ge(b)){let D={};for(let K in b){if(!It(D)&&!n)break;let te=Iw(await b[K](v,a),x,K);te&&(D={...te,...L(K,te.message)},w(te.message),n&&(S[y]=D))}if(!It(D)&&(S[y]={ref:x,...D},!n))return S}}return w(!0),S},gA={mode:_a.onSubmit,reValidateMode:_a.onChange,shouldFocusError:!0};function yA(e={}){let t={...gA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:ka(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ge(t.defaultValues)||Ge(t.values)?mt(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:mt(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},f={...d},m={array:Ow(),state:Ow()},p=t.criteriaMode===_a.all,b=_=>C=>{clearTimeout(c),c=setTimeout(_,C)},y=async _=>{if(!t.disabled&&(d.isValid||f.isValid||_)){let C=t.resolver?It((await N()).errors):await O(n,!0);C!==a.isValid&&m.state.next({isValid:C})}},$=(_,C)=>{!t.disabled&&(d.isValidating||d.validatingFields||f.isValidating||f.validatingFields)&&((_||Array.from(o.mount)).forEach(A=>{A&&(C?Ue(a.validatingFields,A,C):Ze(a.validatingFields,A))}),m.state.next({validatingFields:a.validatingFields,isValidating:!It(a.validatingFields)}))},g=(_,C=[],A,I,q=!0,j=!0)=>{if(I&&A&&!t.disabled){if(i.action=!0,j&&Array.isArray(Y(n,_))){let G=A(Y(n,_),I.argA,I.argB);q&&Ue(n,_,G)}if(j&&Array.isArray(Y(a.errors,_))){let G=A(Y(a.errors,_),I.argA,I.argB);q&&Ue(a.errors,_,G),hA(a.errors,_)}if((d.touchedFields||f.touchedFields)&&j&&Array.isArray(Y(a.touchedFields,_))){let G=A(Y(a.touchedFields,_),I.argA,I.argB);q&&Ue(a.touchedFields,_,G)}(d.dirtyFields||f.dirtyFields)&&(a.dirtyFields=Ho(r,s)),m.state.next({name:_,isDirty:U(_,C),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Ue(s,_,C)},v=(_,C)=>{Ue(a.errors,_,C),m.state.next({errors:a.errors})},x=_=>{a.errors=_,m.state.next({errors:a.errors,isValid:!1})},w=(_,C,A,I)=>{let q=Y(n,_);if(q){let j=Y(s,_,We(A)?Y(r,_):A);We(j)||I&&I.defaultChecked||C?Ue(s,_,C?j:jw(q._f)):te(_,j),i.mount&&y()}},S=(_,C,A,I,q)=>{let j=!1,G=!1,me={name:_};if(!t.disabled){if(!A||I){(d.isDirty||f.isDirty)&&(G=a.isDirty,a.isDirty=me.isDirty=U(),j=G!==me.isDirty);let _e=Wn(Y(r,_),C);G=!!Y(a.dirtyFields,_),_e?Ze(a.dirtyFields,_):Ue(a.dirtyFields,_,!0),me.dirtyFields=a.dirtyFields,j=j||(d.dirtyFields||f.dirtyFields)&&G!==!_e}if(A){let _e=Y(a.touchedFields,_);_e||(Ue(a.touchedFields,_,A),me.touchedFields=a.touchedFields,j=j||(d.touchedFields||f.touchedFields)&&_e!==A)}j&&q&&m.state.next(me)}return j?me:{}},k=(_,C,A,I)=>{let q=Y(a.errors,_),j=(d.isValid||f.isValid)&&Ha(C)&&a.isValid!==C;if(t.delayError&&A?(u=b(()=>v(_,A)),u(t.delayError)):(clearTimeout(c),u=null,A?Ue(a.errors,_,A):Ze(a.errors,_)),(A?!Wn(q,A):q)||!It(I)||j){let G={...I,...j&&Ha(C)?{isValid:C}:{},errors:a.errors,name:_};a={...a,...G},m.state.next(G)}},N=async _=>{$(_,!0);let C=await t.resolver(s,t.context,uA(_||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return $(_),C},E=async _=>{let{errors:C}=await N(_);if(_)for(let A of _){let I=Y(C,A);I?Ue(a.errors,A,I):Ze(a.errors,A)}else a.errors=C;return C},O=async(_,C,A={valid:!0})=>{for(let I in _){let q=_[I];if(q){let{_f:j,...G}=q;if(j){let me=o.array.has(j.name),_e=q._f&&cA(q._f);_e&&d.validatingFields&&$([I],!0);let oa=await Kw(q,o.disabled,s,p,t.shouldUseNativeValidation&&!C,me);if(_e&&d.validatingFields&&$([I]),oa[j.name]&&(A.valid=!1,C))break;!C&&(Y(oa,j.name)?me?vA(a.errors,oa,j.name):Ue(a.errors,j.name,oa[j.name]):Ze(a.errors,j.name))}!It(G)&&await O(G,C,A)}}return A.valid},L=()=>{for(let _ of o.unMount){let C=Y(n,_);C&&(C._f.refs?C._f.refs.every(A=>!qp(A)):!qp(C._f.ref))&&ie(_)}o.unMount=new Set},U=(_,C)=>!t.disabled&&(_&&C&&Ue(s,_,C),!Wn(Kt(),r)),D=(_,C,A)=>rA(_,o,{...i.mount?s:We(C)?r:Qa(_)?{[_]:C}:C},A,C),K=_=>Ip(Y(i.mount?s:r,_,t.shouldUnregister?Y(r,_,[]):[])),te=(_,C,A={})=>{let I=Y(n,_),q=C;if(I){let j=I._f;j&&(!j.disabled&&Ue(s,_,Jw(C,j)),q=gc(j.ref)&&Et(C)?"":C,Qw(j.ref)?[...j.ref.options].forEach(G=>G.selected=q.includes(G.value)):j.refs?Yo(j.ref)?j.refs.forEach(G=>{(!G.defaultChecked||!G.disabled)&&(Array.isArray(q)?G.checked=!!q.find(me=>me===G.value):G.checked=q===G.value||!!q)}):j.refs.forEach(G=>G.checked=G.value===q):Hp(j.ref)?j.ref.value="":(j.ref.value=q,j.ref.type||m.state.next({name:_,values:mt(s)})))}(A.shouldDirty||A.shouldTouch)&&S(_,q,A.shouldTouch,A.shouldDirty,!0),A.shouldValidate&&ot(_)},re=(_,C,A)=>{for(let I in C){if(!C.hasOwnProperty(I))return;let q=C[I],j=_+"."+I,G=Y(n,j);(o.array.has(_)||Ge(q)||G&&!G._f)&&!Pr(q)?re(j,q,A):te(j,q,A)}},Ae=(_,C,A={})=>{let I=Y(n,_),q=o.array.has(_),j=mt(C);Ue(s,_,j),q?(m.array.next({name:_,values:mt(s)}),(d.isDirty||d.dirtyFields||f.isDirty||f.dirtyFields)&&A.shouldDirty&&m.state.next({name:_,dirtyFields:Ho(r,s),isDirty:U(_,j)})):I&&!I._f&&!Et(j)?re(_,j,A):te(_,j,A),zw(_,o)&&m.state.next({...a,name:_}),m.state.next({name:i.mount?_:void 0,values:mt(s)})},Fe=async _=>{i.mount=!0;let C=_.target,A=C.name,I=!0,q=Y(n,A),j=_e=>{I=Number.isNaN(_e)||Pr(_e)&&isNaN(_e.getTime())||Wn(_e,Y(s,A,_e))},G=Fw(t.mode),me=Fw(t.reValidateMode);if(q){let _e,oa,nl=C.type?jw(q._f):XT(_),wn=_.type===Mw.BLUR||_.type===Mw.FOCUS_OUT,Ik=!dA(q._f)&&!t.resolver&&!Y(a.errors,A)&&!q._f.deps||pA(wn,Y(a.touchedFields,A),a.isSubmitted,me,G),id=zw(A,o,wn);Ue(s,A,nl),wn?(!C||!C.readOnly)&&(q._f.onBlur&&q._f.onBlur(_),u&&u(0)):q._f.onChange&&q._f.onChange(_);let od=S(A,nl,wn),Kk=!It(od)||id;if(!wn&&m.state.next({name:A,type:_.type,values:mt(s)}),Ik)return(d.isValid||f.isValid)&&(t.mode==="onBlur"?wn&&y():wn||y()),Kk&&m.state.next({name:A,...id?{}:od});if(!wn&&id&&m.state.next({...a}),t.resolver){let{errors:Dh}=await N([A]);if(j(nl),I){let Hk=Bw(a.errors,n,A),Mh=Bw(Dh,n,Hk.name||A);_e=Mh.error,A=Mh.name,oa=It(Dh)}}else $([A],!0),_e=(await Kw(q,o.disabled,s,p,t.shouldUseNativeValidation))[A],$([A]),j(nl),I&&(_e?oa=!1:(d.isValid||f.isValid)&&(oa=await O(n,!0)));I&&(q._f.deps&&ot(q._f.deps),k(A,oa,_e,od))}},St=(_,C)=>{if(Y(a.errors,C)&&_.focus)return _.focus(),1},ot=async(_,C={})=>{let A,I,q=Vo(_);if(t.resolver){let j=await E(We(_)?_:q);A=It(j),I=_?!q.some(G=>Y(j,G)):A}else _?(I=(await Promise.all(q.map(async j=>{let G=Y(n,j);return await O(G&&G._f?{[j]:G}:G)}))).every(Boolean),!(!I&&!a.isValid)&&y()):I=A=await O(n);return m.state.next({...!Qa(_)||(d.isValid||f.isValid)&&A!==a.isValid?{}:{name:_},...t.resolver||!_?{isValid:A}:{},errors:a.errors}),C.shouldFocus&&!I&&Go(n,St,_?q:o.mount),I},Kt=_=>{let C={...i.mount?s:r};return We(_)?C:Qa(_)?Y(C,_):_.map(A=>Y(C,A))},ba=(_,C)=>({invalid:!!Y((C||a).errors,_),isDirty:!!Y((C||a).dirtyFields,_),error:Y((C||a).errors,_),isValidating:!!Y(a.validatingFields,_),isTouched:!!Y((C||a).touchedFields,_)}),$n=_=>{_&&Vo(_).forEach(C=>Ze(a.errors,C)),m.state.next({errors:_?a.errors:{}})},Ca=(_,C,A)=>{let I=(Y(n,_,{_f:{}})._f||{}).ref,q=Y(a.errors,_)||{},{ref:j,message:G,type:me,..._e}=q;Ue(a.errors,_,{..._e,...C,ref:I}),m.state.next({name:_,errors:a.errors,isValid:!1}),A&&A.shouldFocus&&I&&I.focus&&I.focus()},Ya=(_,C)=>ka(_)?m.state.subscribe({next:A=>"values"in A&&_(D(void 0,C),A)}):D(_,C,!0),ft=_=>m.state.subscribe({next:C=>{fA(_.name,C.name,_.exact)&&mA(C,_.formState||d,W,_.reRenderRoot)&&_.callback({values:{...s},...a,...C,defaultValues:r})}}).unsubscribe,At=_=>(i.mount=!0,f={...f,..._.formState},ft({..._,formState:f})),ie=(_,C={})=>{for(let A of _?Vo(_):o.mount)o.mount.delete(A),o.array.delete(A),C.keepValue||(Ze(n,A),Ze(s,A)),!C.keepError&&Ze(a.errors,A),!C.keepDirty&&Ze(a.dirtyFields,A),!C.keepTouched&&Ze(a.touchedFields,A),!C.keepIsValidating&&Ze(a.validatingFields,A),!t.shouldUnregister&&!C.keepDefaultValue&&Ze(r,A);m.state.next({values:mt(s)}),m.state.next({...a,...C.keepDirty?{isDirty:U()}:{}}),!C.keepIsValid&&y()},ne=({disabled:_,name:C})=>{(Ha(_)&&i.mount||_||o.disabled.has(C))&&(_?o.disabled.add(C):o.disabled.delete(C))},xe=(_,C={})=>{let A=Y(n,_),I=Ha(C.disabled)||Ha(t.disabled);return Ue(n,_,{...A||{},_f:{...A&&A._f?A._f:{ref:{name:_}},name:_,mount:!0,...C}}),o.mount.add(_),A?ne({disabled:Ha(C.disabled)?C.disabled:t.disabled,name:_}):w(_,!0,C.value),{...I?{disabled:C.disabled||t.disabled}:{},...t.progressive?{required:!!C.required,min:Qo(C.min),max:Qo(C.max),minLength:Qo(C.minLength),maxLength:Qo(C.maxLength),pattern:Qo(C.pattern)}:{},name:_,onChange:Fe,onBlur:Fe,ref:q=>{if(q){xe(_,C),A=Y(n,_);let j=We(q.value)&&q.querySelectorAll&&q.querySelectorAll("input,select,textarea")[0]||q,G=iA(j),me=A._f.refs||[];if(G?me.find(_e=>_e===j):j===A._f.ref)return;Ue(n,_,{_f:{...A._f,...G?{refs:[...me.filter(qp),j,...Array.isArray(Y(r,_))?[{}]:[]],ref:{type:j.type,name:_}}:{ref:j}}}),w(_,!1,void 0,j)}else A=Y(n,_,{}),A._f&&(A._f.mount=!1),(t.shouldUnregister||C.shouldUnregister)&&!(WT(o.array,_)&&i.action)&&o.unMount.add(_)}}},Ee=()=>t.shouldFocusError&&Go(n,St,o.mount),pt=_=>{Ha(_)&&(m.state.next({disabled:_}),Go(n,(C,A)=>{let I=Y(n,A);I&&(C.disabled=I._f.disabled||_,Array.isArray(I._f.refs)&&I._f.refs.forEach(q=>{q.disabled=I._f.disabled||_}))},0,!1))},qe=(_,C)=>async A=>{let I;A&&(A.preventDefault&&A.preventDefault(),A.persist&&A.persist());let q=mt(s);if(m.state.next({isSubmitting:!0}),t.resolver){let{errors:j,values:G}=await N();a.errors=j,q=mt(G)}else await O(n);if(o.disabled.size)for(let j of o.disabled)Ze(q,j);if(Ze(a.errors,"root"),It(a.errors)){m.state.next({errors:{}});try{await _(q,A)}catch(j){I=j}}else C&&await C({...a.errors},A),Ee(),setTimeout(Ee);if(m.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:It(a.errors)&&!I,submitCount:a.submitCount+1,errors:a.errors}),I)throw I},Se=(_,C={})=>{Y(n,_)&&(We(C.defaultValue)?Ae(_,mt(Y(r,_))):(Ae(_,C.defaultValue),Ue(r,_,mt(C.defaultValue))),C.keepTouched||Ze(a.touchedFields,_),C.keepDirty||(Ze(a.dirtyFields,_),a.isDirty=C.defaultValue?U(_,mt(Y(r,_))):U()),C.keepError||(Ze(a.errors,_),d.isValid&&y()),m.state.next({...a}))},ia=(_,C={})=>{let A=_?mt(_):r,I=mt(A),q=It(_),j=q?r:I;if(C.keepDefaultValues||(r=A),!C.keepValues){if(C.keepDirtyValues){let G=new Set([...o.mount,...Object.keys(Ho(r,s))]);for(let me of Array.from(G))Y(a.dirtyFields,me)?Ue(j,me,Y(s,me)):Ae(me,Y(j,me))}else{if(Bp&&We(_))for(let G of o.mount){let me=Y(n,G);if(me&&me._f){let _e=Array.isArray(me._f.refs)?me._f.refs[0]:me._f.ref;if(gc(_e)){let oa=_e.closest("form");if(oa){oa.reset();break}}}}if(C.keepFieldsRef)for(let G of o.mount)Ae(G,Y(j,G));else n={}}s=t.shouldUnregister?C.keepDefaultValues?mt(r):{}:mt(j),m.array.next({values:{...j}}),m.state.next({values:{...j}})}o={mount:C.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!C.keepIsValid||!!C.keepDirtyValues,i.watch=!!t.shouldUnregister,m.state.next({submitCount:C.keepSubmitCount?a.submitCount:0,isDirty:q?!1:C.keepDirty?a.isDirty:!!(C.keepDefaultValues&&!Wn(_,r)),isSubmitted:C.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:q?{}:C.keepDirtyValues?C.keepDefaultValues&&s?Ho(r,s):a.dirtyFields:C.keepDefaultValues&&_?Ho(r,_):C.keepDirty?a.dirtyFields:{},touchedFields:C.keepTouched?a.touchedFields:{},errors:C.keepErrors?a.errors:{},isSubmitSuccessful:C.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},Nt=(_,C)=>ia(ka(_)?_(s):_,C),Ir=(_,C={})=>{let A=Y(n,_),I=A&&A._f;if(I){let q=I.refs?I.refs[0]:I.ref;q.focus&&(q.focus(),C.shouldSelect&&ka(q.select)&&q.select())}},W=_=>{a={...a,..._}},Dt={control:{register:xe,unregister:ie,getFieldState:ba,handleSubmit:qe,setError:Ca,_subscribe:ft,_runSchema:N,_focusError:Ee,_getWatch:D,_getDirty:U,_setValid:y,_setFieldArray:g,_setDisabledField:ne,_setErrors:x,_getFieldArray:K,_reset:ia,_resetDefaultValues:()=>ka(t.defaultValues)&&t.defaultValues().then(_=>{Nt(_,t.resetOptions),m.state.next({isLoading:!1})}),_removeUnmounted:L,_disableForm:pt,_subjects:m,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(_){i=_},get _defaultValues(){return r},get _names(){return o},set _names(_){o=_},get _formState(){return a},get _options(){return t},set _options(_){t={...t,..._}}},subscribe:At,trigger:ot,register:xe,handleSubmit:qe,watch:Ya,setValue:Ae,getValues:Kt,reset:Nt,resetField:Se,clearErrors:$n,unregister:ie,setError:Ca,setFocus:Ir,getFieldState:ba};return{...Dt,formControl:Dt}}function Zw(e={}){let t=Bt.default.useRef(void 0),a=Bt.default.useRef(void 0),[n,r]=Bt.default.useState({isDirty:!1,isValidating:!1,isLoading:ka(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:ka(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!ka(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=yA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,nA(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Bt.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Bt.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Bt.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Bt.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Bt.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Bt.default.useEffect(()=>{e.values&&!Wn(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Bt.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=aA(n,s),t.current}var Ww={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},e1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},bA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function ee({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${Q(Ww[a]??Ww.default,e1[n]??e1.md,bA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var Vp="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",$c={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Tt({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${Q(Vp,$c[t]??$c.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function wc({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${Q(Vp,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Gp({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${Q(Vp,$c[a]??$c.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function xA({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${Q("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function bn({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${Q("flex flex-col gap-2",s)}>
      ${e&&l`<${xA} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var $A={google:"Google",github:"GitHub",apple:"Apple"};function wA(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function t1({providers:e,redirectAfter:t}){let a=R();return e.length?l`
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
              href=${wA(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${M} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:$A[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var SA=["google","github","apple"];function a1(){let[e,t]=h.default.useState([]);return h.default.useEffect(()=>{let a=!1;return Ux().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(SA.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function n1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=R(),{theme:s,toggleTheme:i}=uc(),o=a1(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:f}=Zw({defaultValues:{token:e||""}});return l`
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
        <${M} name=${s==="dark"?"sun":"moon"} className="h-4 w-4" />
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
          onSubmit=${d(({token:m})=>n(m))}
        >
          <${bn}
            label=${r("login.tokenLabel")}
            htmlFor="v2-token"
            error=${u.token?.message??""}
            hint=${r("login.tokenHint")}
          >
            <${Tt}
              id="v2-token"
              type="password"
              error=${!!u.token}
              ...${f("token",{required:r("login.tokenRequired"),setValueAs:m=>m.trim()})}
              placeholder=${r("login.tokenPlaceholder")}
              autocomplete="current-password"
            />
          <//>

          ${t&&l`<p
              className=${Q("rounded-[10px] border px-3 py-2 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
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

        <${t1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var r1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},s1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function F({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${Q("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",s1[n]??s1.md,r1[e]??r1.muted,r)}
    >
      ${a&&l`<span
          className=${Q("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var NA=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,i1=/(bash|shell|exec|run|command|terminal|spawn|process)/,o1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function l1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return NA.test(n)?{tone:"danger",key:"tool.riskWrite"}:i1.test(n)?{tone:"warning",key:"tool.riskExec"}:o1.test(n)?{tone:"info",key:"tool.riskNetwork"}:i1.test(r)?{tone:"warning",key:"tool.riskExec"}:o1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}var Sc=480;function _A(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Sc):typeof e=="string"&&e.length>Sc}function u1(e,t){return typeof e!="string"||t||e.length<=Sc?e:`${e.slice(0,Sc).trimEnd()}
...`}function c1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=R(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,f]=h.default.useState(!1),[m,p]=h.default.useState(!1);h.default.useEffect(()=>{p(!1)},[e]);let b=h.default.useMemo(()=>l1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),$=_A(o,c),g=m?"max-h-72":"max-h-36",v=h.default.useCallback(()=>{d&&u?n?.():t?.()},[d,u,n,t]);return l`
    <div className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${M} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${F}
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
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${u1(x.value,m)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${u1(o,m)}</pre>`}

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
  `}function Zs({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:u}){let c=R(),[d,f]=h.default.useState(o),m=h.default.useId(),p=n||a||"";return l`
    <div className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)]">
      <button
        type="button"
        onClick=${()=>f(b=>!b)}
        aria-expanded=${d?"true":"false"}
        aria-controls=${m}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${M} name=${e} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-white">
            ${t||c("authGate.title")}
          </span>
          ${p&&l`<span className="block truncate text-xs text-iron-300">${p}</span>`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[#8fc8f2]">
          ${i&&l`<span className="hidden sm:inline">${i}</span>`}
          <${M}
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
  `}function d1({gate:e,onCancel:t}){let a=R();return l`
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
  `}function m1({gate:e,onCancel:t}){let a=R(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),o=h.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);h.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let u=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=h.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:u}):a("authGate.openAuthorization",{provider:u});return l`
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
          <${M} name="link" className="h-4 w-4" />
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
  `}function f1({gate:e,onSubmit:t,onCancel:a}){let n=R(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),d=h.default.useCallback(async f=>{f.preventDefault();let m=r.trim();if(!m){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(m),s("")}catch(p){o(p?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
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
          <${Tt}
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
  `}var kA="/api/webchat/v2/extensions/pairing/redeem";function p1(e){return V(kA,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Nc({action:e}){let t=R(),a=J(),n=H({mutationFn:({code:u})=>p1(u),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=h.default.useState(""),i=RA(e,t),o=()=>{let u=r.trim();u&&(n.mutate({code:u}),s(""))};return l`
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
        ${CA(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function RA(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function CA(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function EA(e,t){return e?.channel==="slack"&&e.strategy===t}function h1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
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
            <${M} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${EA(e,"inbound_proof_code")?l`<${Nc} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function TA(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Mr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Mr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Mr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Mr.maxTotalBytes}:Mr}function v1(){let e=ya(),t=B({enabled:!!e,queryKey:["session"],queryFn:ac,staleTime:5*6e4});return TA(t.data)}function _c({onSend:e,onCancel:t,disabled:a,canCancel:n=!1,initialText:r="",resetKey:s="",draftKey:i=qo,variant:o="dock",context:u={},statusText:c=""}){let d=R(),f=o==="hero",m=v1(),[p,b]=h.default.useState(()=>Tp(i)),[y,$]=h.default.useState(()=>Dp(i)),[g,v]=h.default.useState(""),[x,w]=h.default.useState(!1),[S,k]=h.default.useState(!1),[N,E]=h.default.useState(!1),O=h.default.useRef(null),L=h.default.useRef(null),U=h.default.useRef([]),D=h.default.useRef(Promise.resolve());h.default.useEffect(()=>{U.current=y},[y]);let K=h.default.useRef(null),te=h.default.useRef(null),re=h.default.useCallback(()=>{te.current&&(window.clearTimeout(te.current),te.current=null);let W=K.current;K.current=null,W&&W.scope===$t()&&Ap(W.key,W.text)},[]),Ae=h.default.useCallback(()=>{te.current&&(window.clearTimeout(te.current),te.current=null),K.current=null},[]),Fe=h.default.useCallback(()=>{let W=O.current;W&&(W.style.height="auto",W.style.height=`${Math.min(W.scrollHeight,200)}px`)},[]);h.default.useEffect(()=>{Fe()},[p,Fe]),h.default.useEffect(()=>(b(Tp(i)),()=>re()),[i,re]);let St=h.default.useRef(i);h.default.useEffect(()=>{if(St.current!==i){St.current=i,$(Dp(i)),v("");return}c$(i,y)},[i,y]),h.default.useEffect(()=>{r&&(b(r),window.requestAnimationFrame(()=>{O.current&&(O.current.focus(),O.current.setSelectionRange(r.length,r.length))}))},[r,s]);let ot=h.default.useCallback(W=>{a||!W||W.length===0||(D.current=D.current.then(async()=>{let{staged:Ye,errors:Dt}=await Xx(W,{limits:m,existing:U.current,t:d});Ye.length>0&&$(_=>{let C=[..._,...Ye];return U.current=C,C}),v(Dt.length>0?Dt.join(" "):"")}).catch(()=>{v(d("chat.attachmentStagingFailed"))}))},[a,m,d]),Kt=h.default.useCallback(W=>{$(Ye=>{let Dt=Ye.filter(_=>_.id!==W);return U.current=Dt,Dt}),v("")},[]),ba=h.default.useCallback(()=>{a||L.current?.click()},[a]),$n=h.default.useCallback(W=>{let Ye=Array.from(W.target.files||[]);ot(Ye),W.target.value=""},[ot]),Ca=h.default.useCallback(async()=>{if(!(!p.trim()||a||x)){w(!0);try{await e(p.trim(),{attachments:y}),b(""),$([]),U.current=[],v(""),Ae(),u$(i),d$(i),O.current&&(O.current.style.height="auto")}catch{}finally{w(!1)}}},[p,y,a,x,e,i,Ae]),Ya=h.default.useCallback(W=>{let Ye=W.target.value;b(Ye),K.current={key:i,text:Ye,scope:$t()},te.current&&window.clearTimeout(te.current),te.current=window.setTimeout(re,300)},[i,re]),ft=h.default.useCallback(async()=>{if(!(!n||S||!t)){k(!0);try{await t()}finally{k(!1)}}},[n,S,t]),At=h.default.useCallback(W=>{W.key==="Enter"&&!W.shiftKey&&(W.preventDefault(),Ca())},[Ca]),ie=h.default.useCallback(W=>{let Ye=Array.from(W.clipboardData?.files||[]);Ye.length>0&&(W.preventDefault(),ot(Ye))},[ot]),ne=h.default.useCallback(W=>{W.preventDefault(),E(!1);let Ye=Array.from(W.dataTransfer?.files||[]);Ye.length>0&&ot(Ye)},[ot]),xe=h.default.useCallback(W=>{W.preventDefault(),!a&&E(!0)},[a]),Ee=h.default.useCallback(W=>{W.currentTarget.contains(W.relatedTarget)||E(!1)},[]),pt=p.trim(),qe=d(f?"chat.heroPlaceholder":"chat.followUpPlaceholder"),Se=m.accept.length>0?m.accept.join(","):void 0,ia=f?"w-full":"px-4 py-3 sm:px-5 lg:px-8",Nt=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",f?"min-h-[120px]":"",a?"opacity-70":""].join(" "),Ir=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",f?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${ia}>
      <div
        className=${Nt}
        onDrop=${ne}
        onDragOver=${xe}
        onDragLeave=${Ee}
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
              <${M} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${y.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${y.map(W=>l`
                <div
                  key=${W.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${W.previewUrl?l`<img
                        src=${W.previewUrl}
                        alt=${W.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${M} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${W.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${W.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>Kt(W.id)}
                    aria-label=${d("chat.attachmentRemove")}
                    title=${d("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${M} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `)}
          </div>
        `}

        <textarea
          ref=${O}
          data-testid="chat-composer"
          value=${p}
          onChange=${Ya}
          onKeyDown=${At}
          onPaste=${ie}
          placeholder=${qe}
          rows=${1}
          disabled=${a}
          className=${Ir}
        />

        <input
          ref=${L}
          type="file"
          multiple
          accept=${Se}
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
              onClick=${ba}
              disabled=${a}
              aria-label=${d("chat.attachFiles")}
              title=${d("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${M} name="plus" className="h-5 w-5" />
            </button>
            ${n?l`
                <${T}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${ft}
                  disabled=${S}
                  aria-label=${d("common.cancel")}
                  title=${d("common.cancel")}
                  className="rounded-full"
                >
                  <${M} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${T}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${Ca}
                  disabled=${a||x||!pt}
                  aria-label=${d("chat.send")}
                  className="rounded-full"
                >
                  <${M} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `}var g1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function y1({status:e}){let t=R();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",g1[e]||g1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function b1({onSuggestion:e,onSend:t,disabled:a,initialText:n,resetKey:r,draftKey:s,context:i,statusText:o,canCancel:u,onCancel:c}){let d=R(),f=[{icon:"tool",title:d("chat.suggestion1"),detail:d("chat.suggestion1Desc")},{icon:"shield",title:d("chat.suggestion2"),detail:d("chat.suggestion2Desc")},{icon:"plug",title:d("chat.suggestion3"),detail:d("chat.suggestion3Desc")}];return l`
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
        <${_c}
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
                <${M} name=${m.icon} className="h-4 w-4" />
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
  `}var AA=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function x1({open:e,onClose:t}){let a=R();return e?l`
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
          ${AA.map((n,r)=>l`
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
  `:null}function w1(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let u=$1([o]);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}if(DA(o)){let u=$1(o.toolCalls);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function $1(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function DA(e){return e.toolCalls&&e.toolCalls.length>0}var S1=!1;function MA(){S1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),S1=!0)}function N1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}MA();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var Yp=360;function OA(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",Ys("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>Yp){t.style.maxHeight=`${Yp}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${Yp}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function LA({content:e,className:t=""}){let a=h.default.useRef(null),n=h.default.useMemo(()=>N1(e),[e]);return h.default.useEffect(()=>{OA(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var na=h.default.memo(LA);var _1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},PA={success:"ok",declined:"declined",error:"err",running:"run"},UA=2;function Ws({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${FA} tools=${e.toolCalls} />`:l`<${qA} activity=${e} />`}function jA(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function FA({tools:e}){let t=R(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=h.default.useState(n);if(h.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=UA)return l`
      <div className="flex flex-col gap-3">
        ${e.map((o,u)=>l`<${Ws}
            key=${o.id||o.callId||`${o.toolName}-${u}`}
            activity=${o}
          />`)}
      </div>
    `;let i=jA(t,e);return l`
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
          ${e.map((o,u)=>l`<${Ws}
              key=${o.id||o.callId||`${o.toolName}-${u}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function qA({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=h.default.useState(n==="error"||n==="declined");h.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let f=_1[n]||_1.running,m=i!=null,p=h.default.useId(),b=l`
    <button
      type="button"
      onClick=${()=>d(y=>!y)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${p}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",f].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${PA[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${r&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${r}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${m&&l`<span className="font-mono text-[11px] text-iron-300">${i}ms</span>`}
        <${M}
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
          <${M} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${t?"min-w-0 flex-1":"min-w-0 max-w-[85%] flex-1"}>
        ${b}
        ${c&&l`<${zA}
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
  `}function zA({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=R(),u=h.default.useMemo(()=>{let m=[];return r&&m.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&m.push({id:"details",label:o("tool.tabDetails")}),a&&m.push({id:"params",label:o("tool.tabParameters")}),n&&m.push({id:"result",label:o("tool.tabResult")}),m},[o,r,t,a,n,s]),[c,d]=h.default.useState(null),f=c&&u.some(m=>m.id===c)?c:u[0]?.id;return h.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),u.length===0?l`
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
          ${o(s==="error"?"tool.exitError":s==="declined"?"tool.exitDeclined":s==="running"?"tool.exitRunning":"tool.exitOk")}${i!==null?` \xB7 ${i}ms`:""}
        </span>
      </div>
      <div className="p-3 text-xs">
        ${f==="details"&&l`<div className="whitespace-pre-wrap text-iron-200">${t}</div>`}
        ${f==="params"&&l`<pre className="overflow-x-auto rounded bg-iron-900 p-2 font-mono text-iron-100">${a}</pre>`}
        ${f==="result"&&l`<${BA} text=${n} />`}
        ${(f==="error"||f==="declined")&&l`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",f==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function BA({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(IA)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
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
                  >${KA(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function IA(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function KA(e){return e==null?"":String(e)}function k1({activity:e}){let t=w1(e),a=VA(e),[n,r]=h.default.useState(a);return h.default.useEffect(()=>{a&&r(!0)},[a]),l`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${()=>r(s=>!s)}
        aria-expanded=${n?"true":"false"}
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
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((s,i)=>l`
            <${HA}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function HA({item:e}){if(e.role==="thinking")return l`<${QA} content=${e.content} />`;if(e.role==="tool_activity"||Jp(e)){let t=Jp(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${Ws} activity=${t} />`}return null}function QA({content:e}){return e?l`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${M} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${na} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function Jp(e){return e?.toolCalls&&e.toolCalls.length>0}function VA(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:Jp(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function kc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function GA({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=h.default.useState(()=>t&&e.preview_url||null);return h.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return sc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${M} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var R1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",C1="px-3 py-2";function Rc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Na(e.fetch_url);kc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${GA} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${R1} ${C1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${R1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${C1} text-left transition-colors hover:bg-iron-900/80`}
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
  </div>`}var E1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function ei({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return h.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),h.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${Q("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",E1[n]??E1.md,r)}
      >
        ${a?l`<${Xp} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function Xp({children:e,onClose:t,className:a=""}){return l`
    <div
      className=${Q("flex shrink-0 items-center justify-between gap-4","px-5 py-4 md:px-7 md:py-5","border-b border-[var(--v2-panel-border)]",a)}
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
  `}function ti({children:e,className:t=""}){return l`
    <div className=${Q("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function ai({children:e,className:t=""}){return l`
    <div
      className=${Q("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var T1=1e5;function Cc({attachment:e,onClose:t}){let a=!!e,[n,r]=h.default.useState("loading"),[s,i]=h.default.useState({}),o=e?Jx(e.mime_type):"download";if(h.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Na(e.fetch_url).then(async f=>{d=URL.createObjectURL(f);let m={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")m.dataUrl=await $p(f);else if(o==="pdf")m.frameUrl=d;else if(o==="text"){let p=await f.text();m.truncated=p.length>T1,m.text=m.truncated?p.slice(0,T1):p}if(c){URL.revokeObjectURL(d);return}i(m),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${ei} open=${a} onClose=${t} size="xl">
      <${Xp} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${ti} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${YA} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${ai}>
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
  `}function YA({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
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
      </div>`}}var JA=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function XA(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function A1(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of XA(e).matchAll(JA)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function D1(e){return e.split("/").filter(Boolean).pop()||e}function M1(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function ZA({threadId:e,path:t,onPreview:a}){let[n,r]=h.default.useState({mime_type:"",size_label:""});h.default.useEffect(()=>{let i=!0;return yx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:M1(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:D1(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:rc({threadId:e,path:t})};return l`<${Rc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function O1({threadId:e,content:t}){let a=h.default.useMemo(()=>A1(t),[t]),[n,r]=h.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${ZA}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${Cc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var L1={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function WA(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function e4({content:e}){let[t,a]=h.default.useState(!1);return e?l`
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
          <${na} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function t4({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:f,timestamp:m}=e,p=n==="user",[b,y]=h.default.useState(!1),[$,g]=h.default.useState(null),v=h.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),Ys("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||f&&f.length>0){let L=f&&f.length>0?{id:e.id,toolCalls:f}:e;return l`<${Ws} activity=${L} />`}if(n==="thinking")return l`<${e4} content=${r} />`;if(n==="image")return l`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((U,D)=>U.data_url?l`<img key=${D} src=${U.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:l`
                  <div key=${D} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${U.path&&l`<div className="mt-1 font-mono text-xs text-iron-300">${U.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let x=WA(m),w=n==="user"||n==="assistant"&&!u,S=n==="system"||n==="error",k=p?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",N=p?"":"w-full min-w-0 max-w-full",E=c==="error"&&t,O=w||E||x;return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",p?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",k].join(" ")}>
        <div
          className=${["text-base leading-7",N,L1[n]||L1.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${na} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((L,U)=>l`<img key=${U} src=${L} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((L,U)=>l`<${Rc}
                key=${L.id||U}
                att=${L}
                onPreview=${g}
              />`)}
            </div>
            <${Cc}
              attachment=${$}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&l`<${O1}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${O&&l`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",p?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${x&&l`<time dateTime=${m} className="shrink-0 font-mono text-[11px] text-iron-500">${x}</time>`}
          ${(w||E)&&l`
            <div className="flex shrink-0 items-center gap-1">
            ${w&&l`
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
            ${E&&l`
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
  `}var P1=h.default.memo(t4);function B1(e){let t=a4(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(I1(r)){let s=U1(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){j1(a,s),F1(a,r),n+=s.length;continue}}if(Zp(r)){let s=U1(t,n);j1(a,s),n+=s.length-1;continue}F1(a,r)}return a}function a4(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Ec(i);o&&I1(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!Zp(i))continue;let o=Ec(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function U1(e,t){let a=t,n=Ec(e[t]);for(;a<e.length&&Zp(e[a])&&n4(n,e[a]);)a+=1;return e.slice(t,a)}function n4(e,t){let a=Ec(t);return!e||!a||a===e}function j1(e,t){if(t.length===0)return;let a=r4(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function F1(e,t){e.push({type:"message",id:t.id,message:t})}function I1(e){return e.role==="assistant"&&!K1(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function Zp(e){return e.role==="thinking"||e.role==="tool_activity"||K1(e)}function K1(e){return e?.toolCalls&&e.toolCalls.length>0}function Ec(e){return e?.turnRunId||null}function r4(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:s4(t,a))}function s4(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=q1(z1(e.updatedAt||e.timestamp),z1(t.updatedAt||t.timestamp));return a!==0?a:q1(e.sequence,t.sequence)}function q1(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function z1(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}function H1({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=R(),c=h.default.useRef(null),d=h.default.useRef(!0),[f,m]=h.default.useState(!0);h.default.useEffect(()=>{if(!c.current||!d.current)return;let g=window.requestAnimationFrame(()=>{let v=c.current;v&&(v.scrollTop=v.scrollHeight)});return()=>window.cancelAnimationFrame(g)},[e,i]);let p=h.default.useCallback(()=>{let $=c.current;if(!$)return;let g=100,v=$.scrollHeight-$.scrollTop-$.clientHeight;d.current=v<g,m(v<g),a&&$.scrollTop<g&&n&&!t&&n()},[a,n,t]),b=h.default.useCallback(()=>{let $=c.current;$&&($.scrollTop=$.scrollHeight,d.current=!0,m(!0))},[]),y=h.default.useMemo(()=>B1(e),[e]);return l`
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
        ${y.map($=>$.type==="activity-run"?l`<${k1} key=${$.id} activity=${$.activity} />`:l`<${P1}
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
        <${M} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function Q1({notice:e,onRecover:t}){return l`
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
  `}function V1({suggestions:e,onSelect:t}){return!e||e.length===0?null:l`
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
  `}function G1(){return l`
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
  `}function Tc(){return V("/api/webchat/v2/channels/connectable")}function Y1(e,t){if(!Wp(e))return null;let a=Ac(e),n=u4(a),r=null;for(let s of t||[]){if(!l4(s))continue;let i=c4(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function Wp(e){let t=Ac(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function i4(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function o4(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>J1(Ac(n))):a}function l4(e){return e?.strategy!=="admin_managed_channels"}function u4(e){return X1(e,"slack")&&J1(e)}function J1(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function Ac(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function c4(e,t,a={}){return(a.commandAliasesOnly?o4(t,{channelManagementOnly:!0}):i4(t)).reduce((r,s)=>{let i=Ac(s);return X1(e,i)?Math.max(r,i.length):r},0)}function X1(e,t){return t?` ${e} `.includes(` ${t} `):!1}function Z1(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return d4(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function W1(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return{...a,kind:"gate"}}function d4(e,t,a){if(!t)return e;let n=m4(t);return{...e,toolName:t.tool_name||null,description:t.reason||a,actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function m4(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function e2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function t2(){return{terminalByInvocation:new Map}}function a2(e){e?.current?.terminalByInvocation?.clear()}function th(e,t,a){let n=r2(t,{toolStatus:"running"});n&&ni(e,n,a)}function n2(e,t,a,n="gate_declined"){let r=r2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&ni(e,r,a)}function ni(e,t,a){if(!t)return;let n=y4(t);n=g4(n,a),e(r=>{let s=s2(n),i=p4(r,n,s);if(i>=0){let u=[...r];return u[i]=h4(u[i],n),eh(u[i],a),u}let o={id:s,role:"tool_activity",...n};return eh(o,a),[...r,o]})}function r2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||f4(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Uo(i)||i,toolStatus:t.toolStatus||"running",toolDetail:null,toolParameters:null,toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function f4(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function s2(e){return`tool-${e.invocationId}`}function p4(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function h4(e,t){let a=Po(e.toolStatus),n=Po(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:v4(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=s2(t),i.gateActivity=!1),i}function v4(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function g4(e,t){if(!e?.invocationId)return e;if(Po(e.toolStatus))return eh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function eh(e,t){!e?.invocationId||!Po(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function y4(e){let t=Uo(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function c2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:u}){let c=h.default.useRef(new Set),d=h.default.useRef(null),f=h.default.useRef(null);return h.default.useCallback(m=>{let{type:p,frame:b}=m||{};if(!(!p||!b))switch(p){case"accepted":{let y=b.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=b.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.($=>$&&$.runId===y.turn_run_id?{...$,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),b4(n,y.turn_run_id,f)),a(!0);return}case"capability_activity":{let y=b.activity;if(!y||!y.invocation_id)return;ni(t,Cp(y),o);return}case"capability_display_preview":{let y=b.preview;if(!y||!y.invocation_id)return;let $=Rp(y);ni(t,$,o);return}case"gate":case"auth_required":{let y=Z1(p,b.prompt);y&&(th(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=b.reply||{};t($=>[...$,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=b.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Oc(c,u,y,!1);return}case"failed":{let y=b.run_state||{},$=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),nh(t,{runId:$,status:y.status||"failed",failureCategory:S4(y),failureSummary:null}),Oc(c,u,$,!1);return}case"projection_snapshot":case"projection_update":{let y=b.state?.items||[];$4({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:u,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:f,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,u])}function Oc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var i2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),o2=new Set(["completed","succeeded"]),Dc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Mc=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function l2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function b4(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function x4(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Mc.has(o);let u=e?.current,c=u?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&u?.status&&!Mc.has(u.status)?!0:!u?.runId||!u.status?!1:!Mc.has(u.status)}function $4({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:f,toolActivityStateRef:m}){let p=new Map,b=new Set,y=d?.current||null,$=y?.runId||u?.current||null;for(let v of e){let x=v.run_status;x?.run_id&&x.status&&(p.set(x.run_id,x.status),$&&$!==x.run_id&&y?.status&&!i2.has(y.status)&&Dc.has(x.status)&&b.add(x.run_id))}let g=u?.current??null;for(let v of e){if(v.run_status){let{run_id:x,status:w,failure_category:S,failure_summary:k}=v.run_status,N=i2.has(w),E=d?.current?.source==="local"?d.current.runId:null,O=!!(x&&E&&E!==x),L=g??u?.current??null,U=!!(N&&x&&L&&L!==x),D=x&&Dc.has(w)?u2(f,x):null;if(x&&b.has(x)||O)continue;if(U){u2(f,d?.current?.runId)?.outcome==="resumed"&&(w4({runId:x,activePromptRunId:d?.current?.runId,success:o2.has(w),status:w,failureCategory:S,failureSummary:k,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,locallyResolvedGatesRef:f}),g=null);continue}if(D){l2(r,x,c),D.outcome==="resumed"?(n(!0),s?.(K=>K&&K.runId===x?{...K,status:K.status==="awaiting_gate"?"queued":K.status||"queued"}:{runId:x,threadId:t,status:"queued"}),g=x,u&&(u.current=x)):(n(!1),d?.current?.runId===x&&s?.(null),g=null,u?.current===x&&(u.current=null));continue}x&&(g=x,!N&&u&&(u.current=x),s?.(K=>K&&K.runId===x?{...K,status:w}:{runId:x,threadId:t,status:w})),x&&Dc.has(w)?c&&(c.current=x):x&&c?.current===x&&(c.current=null),N?(n(!1),r(null),s?.(null),ah(f,x),g=null,u&&(u.current=null),x&&c?.current===x&&(c.current=null),Oc(o,i,x,o2.has(w)),(w==="failed"||w==="recovery_required")&&nh(a,{runId:x,status:w,failureCategory:S,failureSummary:k})):Dc.has(w)||(l2(r,x,c),ah(f,x),n(!0))}if(v.text){let x=`text-${v.text.id}`;a(w=>{let S=w.findIndex(N=>N.id===x),k={id:x,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(S>=0){let N=[...w];return N[S]=k,N}return[...w,k]}),n(!1)}if(v.thinking){let x=`thinking-${v.thinking.id}`;a(w=>{let S=w.findIndex(N=>N.id===x),k={id:x,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let N=[...w];return N[S]=k,N}return[...w,k]})}if(v.capability_activity){let x=v.capability_activity;x.invocation_id&&ni(a,Cp(x),m)}if(v.gate){let x=W1(v.gate),w=x?.runId||null;w&&!x4(d,x,p,u,b,c)&&!_4(f,w,x.gateRef)&&(th(a,x,m),r(S=>S||x),s?.(S=>S&&S.runId===w?{...S,status:Mc.has(S.status)?S.status:"awaiting_gate"}:{runId:w,threadId:t,status:"awaiting_gate"}),c&&(c.current=w),n(!1))}if(v.skill_activation){let{id:x,skill_names:w=[],feedback:S=[]}=v.skill_activation;if(w.length||S.length){let k=`skill-${x||w.join("-")||"activation"}`,N=[w.length?`Skill activated: ${w.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(E=>E.some(O=>O.id===k)?E:[...E,{id:k,role:"system",content:N,timestamp:new Date().toISOString()}])}}}u&&g&&(u.current=g)}function w4({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:u,setActiveRun:c,onRunSettled:d,settledRunsRef:f,latestRunIdRef:m,promptRunIdRef:p,locallyResolvedGatesRef:b}){o(!1),u(null),c?.(null),ah(b,t),m&&(m.current=null),p?.current===t&&(p.current=null),Oc(f,d,e,a),(n==="failed"||n==="recovery_required")&&nh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function S4(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function nh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=e2({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString()}]})}function u2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return N4(r);return null}function N4(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function ah(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function _4(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function d2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function m2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function f2(e,t,a,n){let r=R4(n);return r?(k4(e,t,a,{timelineMessageId:r}),r):null}function k4(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function R4(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var C4=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function p2({threadId:e,onEvent:t,enabled:a}){let[n,r]=h.default.useState("idle"),s=h.default.useRef(t);s.current=t;let i=h.default.useRef(null);return h.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function f(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Mx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);u=setTimeout(f,y)};let b=(y,$)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||$,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>b(y,"message");for(let y of C4)o.addEventListener(y,$=>b($,y))}function m(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function p(){document.visibilityState==="hidden"?m():o||f()}return f(),document.addEventListener("visibilitychange",p),()=>{document.removeEventListener("visibilitychange",p),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var E4=3e4,T4="credential_stored_gate_resolution_failed",A4="ironclaw-product-auth",rh="ironclaw:product-auth:oauth-complete",D4="ironclaw:product-auth:oauth-complete";async function h2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),E4);try{return await e(t.signal)}finally{clearTimeout(a)}}function M4(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=T4,t.cause=e,t}function O4(e){let a=Ct.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function L4(e){return e?.continuation?.type==="turn_gate_resume"}function P4(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function v2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function U4(e){return e?.type===D4&&e?.status==="completed"}function j4(e,t,a){if(!U4(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function sh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function F4(e){if(!Wp(e))return null;try{let a=(await Ct.fetchQuery({queryKey:["connectable-channels"],queryFn:Tc}))?.channels||[];return Y1(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function g2(e){let t=h.default.useRef(new Map),a=h.default.useRef(1),[n,r]=h.default.useState(0),[s,i]=h.default.useState(Date.now()),[o,u]=h.default.useState(null),c=h.default.useRef(o),d=h.default.useCallback(ie=>{let ne=typeof ie=="function"?ie(c.current):ie;c.current=ne,u(ne)},[]);h.default.useEffect(()=>{c.current=o},[o]);let[f,m]=h.default.useState(null),p=h.default.useCallback(()=>t.current.get(e||"__new__")||[],[e]),b=h.default.useCallback(ie=>{let ne=e||"__new__";ie.length>0?t.current.set(ne,ie):t.current.delete(ne)},[e]),{messages:y,hasMore:$,nextCursor:g,isLoading:v,loadError:x,loadHistory:w,seedThreadMessages:S,setMessages:k}=o$(e,{getPendingMessages:p,setPendingMessages:b}),[N,E]=h.default.useState(!1),[O,L]=h.default.useState(null),[U,D]=h.default.useState(e),K=h.default.useRef(t2()),te=h.default.useRef(new Map),re=h.default.useRef({gateKey:null,credentialRef:null,inFlight:!1});U!==e&&(D(e),E(!1),L(null),u(null),m(null)),h.default.useEffect(()=>{a2(K),te.current.clear()},[e]);let Ae=Math.max(0,Math.ceil((n-s)/1e3)),Fe=O?.runId&&O?.gateRef?`${O.runId}
${O.gateRef}`:null;h.default.useEffect(()=>{if(!n)return;let ie=setInterval(()=>i(Date.now()),250);return()=>clearInterval(ie)},[n]),h.default.useEffect(()=>{re.current.gateKey!==Fe&&(re.current={gateKey:Fe,credentialRef:null,inFlight:!1})},[Fe]),h.default.useEffect(()=>{if(!v2(O))return;let ie=Date.now(),ne=qe=>{j4(qe,O,ie)&&(L(Se=>v2(Se)?null:Se),E(!0))},xe=null;typeof window.BroadcastChannel=="function"&&(xe=new window.BroadcastChannel(A4),xe.onmessage=qe=>ne(qe.data));let Ee=qe=>{qe.key===rh&&ne(sh(qe.newValue))};window.addEventListener("storage",Ee),ne(sh(window.localStorage?.getItem?.(rh)));let pt=window.setInterval(()=>{ne(sh(window.localStorage?.getItem?.(rh)))},500);return()=>{window.clearInterval(pt),xe&&xe.close(),window.removeEventListener("storage",Ee)}},[O]);let St=c2({threadId:e,setMessages:k,setIsProcessing:E,setPendingGate:L,setActiveRun:d,activeRunRef:c,locallyResolvedGatesRef:te,toolActivityStateRef:K,onRunSettled:(ie,{success:ne})=>{ne&&b([]),w(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:ie&&ne?{[ie]:new Date().toISOString()}:null})}}),{status:ot}=p2({threadId:e,onEvent:St,enabled:!!e}),Kt=h.default.useCallback(async(ie,ne={})=>{let{threadId:xe,attachments:Ee=[]}=ne,pt=Ee.map(Zx),qe=Ee.map(Wx);if(Ee.length===0){let A=await F4(ie);if(A)return m(A),{channel_connect_action:A}}m(null);let Se=xe||e;if(!Se){let A=await nc();if(Ct.invalidateQueries({queryKey:["threads"]}),Se=A?.thread?.thread_id,!Se)throw new Error("createThread returned no thread_id")}let ia=Se,Nt={id:`pending-${a.current++}`,role:"user",content:ie,attachments:qe,timestamp:new Date().toISOString(),isOptimistic:!0},Ir={id:Nt.id,role:"user",content:ie,attachments:qe,timestamp:Nt.timestamp,isOptimistic:!0};d2(t.current,ia,Nt);let W=Nt.id,Ye=!e||Se===e,Dt=A=>{Ye&&k(A)},_=A=>{Se!==e&&S(Se,A)},C=A=>{Ye&&A()};Dt(A=>[...A,Ir]),Se!==e&&S(Se,A=>[...A,Ir]),C(()=>{E(!0),L(null)});try{let A=await Tx({threadId:Se,content:ie,attachments:pt});O4(Se)&&Ct.invalidateQueries({queryKey:["threads"]}),A?.run_id&&Ye&&d({runId:A.run_id,threadId:A.thread_id||Se,status:A.status||null,source:"local"});let I=f2(t.current,ia,W,A?.accepted_message_ref);if(I){let q=j=>j.map(G=>G.id===W?{...G,timelineMessageId:I}:G);Dt(q),_(q)}if(A?.outcome==="rejected_busy"){let q=j=>j.map(G=>G.id===W?{...G,isOptimistic:!1,status:"error"}:G);if(Dt(q),_(q),A?.notice){let j={id:`system-rejected-${a.current++}`,role:"system",content:A.notice,timestamp:new Date().toISOString(),isOptimistic:!1},G=me=>[...me,j];Dt(G),_(G)}C(()=>E(!1))}return A}catch(A){A.status===429&&r(Date.now()+z4(A));let I=q=>q.map(j=>j.id===W?{...j,isOptimistic:!1,status:"error",error:A.message}:j);throw Dt(I),_(I),C(()=>E(!1)),A}finally{m2(t.current,ia,W)}},[e,k,S]),ba=h.default.useCallback(async(ie,ne={})=>{if(!O)return;let{runId:xe,gateRef:Ee}=O;if(!xe||!Ee)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let pt=await wp({threadId:e,runId:xe,gateRef:Ee,resolution:ie,always:ne.always,credentialRef:ne.credentialRef}),qe=P4(pt);if(te.current.set(`${xe}
${Ee}`,{resolution:ie,outcome:qe}),q4(ie)&&qe==="resumed"&&n2(k,O,K),L(null),qe==="resumed"){E(!0),d({runId:pt?.run_id||xe,threadId:pt?.thread_id||e,status:pt?.status||"queued"});return}E(!1),d(null)},[O,e,k,d]),$n=h.default.useCallback(async ie=>{if(!O)throw new Error("auth gate is no longer pending");let{runId:ne,gateRef:xe,provider:Ee}=O;if(!ne||!xe||!Ee)throw new Error("auth gate is missing required credential metadata");let pt=O.accountLabel||`${Ee} credential`,qe=`${ne}
${xe}`;if(re.current.gateKey!==qe&&(re.current={gateKey:qe,credentialRef:null,inFlight:!1}),re.current.inFlight)throw new Error("auth token submission already in progress");re.current.inFlight=!0;try{let Se=re.current.credentialRef,ia=null;if(!Se){if(ia=await h2(Nt=>Lx({provider:Ee,accountLabel:pt,token:ie,threadId:e,runId:ne,gateRef:xe,signal:Nt})),Se=ia?.credential_ref,!Se)throw new Error("manual token submit returned no credential_ref");re.current.credentialRef=Se}if(!L4(ia))try{await h2(Nt=>wp({threadId:e,runId:ne,gateRef:xe,resolution:"credential_provided",credentialRef:Se,signal:Nt}))}catch(Nt){throw M4(Nt)}re.current={gateKey:null,credentialRef:null,inFlight:!1},L(null),E(!0)}catch(Se){throw re.current.gateKey===qe&&(re.current.inFlight=!1),Se}},[O,e]),Ca=h.default.useCallback(async ie=>{let ne=o?.runId;!ne||!e||(L(null),E(!1),d(null),await Ox({threadId:e,runId:ne,reason:ie}))},[o,e]),Ya=h.default.useCallback(()=>{$&&g&&w(g)},[$,g,w]),ft=h.default.useCallback(async(ie,ne,xe)=>{let Ee="approved",pt=!1;ne==="deny"?Ee="denied":ne==="cancel"?Ee="cancelled":ne==="always"&&(Ee="approved",pt=!0),await ba(Ee,{always:pt})},[ba]),At=h.default.useCallback(()=>{},[]);return{messages:y,isProcessing:N,pendingGate:O,channelConnectAction:f,activeRun:o,sseStatus:ot,historyLoading:v,historyLoadError:x,hasMore:$,cooldownSeconds:Ae,send:Kt,resolveGate:ba,submitAuthToken:$n,cancelRun:Ca,loadMore:Ya,dismissChannelConnectAction:()=>m(null),suggestions:[],setSuggestions:At,retryMessage:At,approve:ft,recoverHistory:At,recoveryNotice:null}}function q4(e){return e==="denied"||e==="cancelled"}function z4(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function y2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}var B4=1500;function b2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let{messages:o,isProcessing:u,pendingGate:c,channelConnectAction:d,suggestions:f,sseStatus:m,historyLoading:p,historyLoadError:b,hasMore:y,cooldownSeconds:$,recoveryNotice:g,activeRun:v,send:x,cancelRun:w,retryMessage:S,approve:k,recoverHistory:N,loadMore:E,setSuggestions:O,submitAuthToken:L,dismissChannelConnectAction:U}=g2(t),D=h.default.useMemo(()=>e.find(ft=>ft.id===t)||null,[e,t]),K=h.default.useMemo(()=>y2({gatewayStatus:i,activeThread:D}),[i,D]),te=o.length>0||u||!!c||!!d,re=!p&&!te&&!b,Ae=u&&!c||$>0,Fe=$>0?`Retry in ${$}s`:void 0,St=t||qo,ot=!!(t&&v?.runId&&v.threadId===t&&u&&!c),Kt=h.default.useCallback(async(ft,{images:At=[],attachments:ie=[]}={})=>{let ne=await x(ft,{images:At,attachments:ie,threadId:t}),xe=ne?.thread_id||t;return!t&&xe&&a&&a(xe,{replace:!0}),ne},[t,a,x]),ba=h.default.useCallback(async ft=>{O([]),await Kt(ft)},[Kt,O]),$n=h.default.useCallback(()=>w("user_requested"),[w]);h.default.useEffect(()=>{if(!t)return;if(c){fc(t,gn.NEEDS_ATTENTION);return}if(u){fc(t,gn.RUNNING);return}let ft=setTimeout(()=>hw(t),B4);return()=>clearTimeout(ft)},[t,c,u]);let[Ca,Ya]=h.default.useState(!1);return h.default.useEffect(()=>{let ft=At=>{if(At.key==="Escape"){Ya(!1);return}if(At.key!=="?")return;let ie=At.target,ne=ie?.tagName;ne==="INPUT"||ne==="TEXTAREA"||ie?.isContentEditable||(At.preventDefault(),Ya(xe=>!xe))};return window.addEventListener("keydown",ft),()=>window.removeEventListener("keydown",ft)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${y1} status=${m} />

        ${b&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${b}
          </div>
        `}

        ${re&&l`
          <${b1}
            onSuggestion=${ba}
            onSend=${Kt}
            disabled=${Ae}
            initialText=${r}
            resetKey=${s}
            draftKey=${St}
            context=${K}
            statusText=${Fe}
            canCancel=${ot}
            onCancel=${$n}
          />
        `}
        ${!re&&l`
          <${H1}
            messages=${o}
            isLoading=${p}
            hasMore=${y}
            onLoadMore=${E}
            onRetryMessage=${S}
            threadId=${t}
            pending=${u}
          >
            ${g&&l`
              <${Q1}
                notice=${g}
                onRecover=${N}
              />
            `}
            ${u&&!c&&l`<${G1} />`}
            ${d&&l`
              <${h1}
                connectAction=${d}
                onDismiss=${U}
              />
            `}
            ${c&&(c.kind==="auth_required"?c.challengeKind==="oauth_url"?l`
                  <${m1}
                    gate=${c}
                    onCancel=${()=>k(c.requestId,"cancel",c.kind)}
                  />
                `:c.challengeKind==="manual_token"?l`
                  <${f1}
                    gate=${c}
                    onSubmit=${L}
                    onCancel=${()=>k(c.requestId,"cancel",c.kind)}
                  />
                `:l`
                  <${d1}
                    gate=${c}
                    onCancel=${()=>k(c.requestId,"cancel",c.kind)}
                  />
                `:l`
              <${c1}
                gate=${c}
                onApprove=${()=>k(c.requestId,"approve",c.kind)}
                onDeny=${()=>k(c.requestId,"deny",c.kind)}
                onAlways=${()=>k(c.requestId,"always",c.kind)}
              />
            `)}
          <//>

          <${V1}
            suggestions=${f}
            onSelect=${ba}
          />

          <${_c}
            onSend=${Kt}
            disabled=${Ae}
            initialText=${r}
            resetKey=${s}
            draftKey=${St}
            context=${K}
            statusText=${Fe}
            canCancel=${ot}
            onCancel=${$n}
          />
        `}
      </div>
      <${x1}
        open=${Ca}
        onClose=${()=>Ya(!1)}
      />
    </div>
  `}function ih(){let{threadsState:e,gatewayStatus:t}=Ba(),{threadId:a}=st(),n=de(),r=Pe(),s=r.state?.composerDraft||"";h.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=h.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${b2}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function x2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?Vs(e,t):"",model:e?dc(e,t):""}}function $2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=h.default.useState(()=>x2(e,a)),[f,m]=h.default.useState(""),[p,b]=h.default.useState([]),[y,$]=h.default.useState(null),[g,v]=h.default.useState(""),x=h.default.useRef(!!e);h.default.useEffect(()=>{n&&(d(x2(e,a)),m(""),b([]),$(null),v(""),x.current=!!e)},[n,e,a]);let w=e?.builtin===!0,S=e&&!e.builtin,k=h.default.useCallback((U,D)=>{d(K=>{let te={...K,[U]:D};return U==="name"&&!x.current&&(te.id=H$(D)),te})},[]),N=h.default.useCallback(()=>!w&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!w&&!Q$(c.id.trim())?u("llm.invalidId"):!S&&!w&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,w,S,u]),E=h.default.useCallback(async()=>{let U=N();if(U){$({tone:"error",text:U});return}v("save");try{await s({form:c,apiKey:f,provider:e}),r()}catch(D){$({tone:"error",text:D.message})}finally{v("")}},[f,c,r,s,e,N]),O=h.default.useCallback(async()=>{if(!c.model.trim()){$({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let U=await i(Lp(e,c,f,a));$({tone:U.ok?"success":"error",text:U.message})}catch(U){$({tone:"error",text:U.message})}finally{v("")}},[f,a,c,i,e,u]),L=h.default.useCallback(async()=>{if((w?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){$({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let D=await o(Lp(e,c,f,a));if(!D.ok||!Array.isArray(D.models)||!D.models.length)$({tone:"error",text:D.message||u("llm.modelsFetchFailed")});else{b(D.models);let K=V$(c.model,D.models);K!==null&&k("model",K),$({tone:"success",text:u("llm.modelsFetched",{count:D.models.length})})}}catch(D){$({tone:"error",text:D.message})}finally{v("")}},[f,a,c,w,o,e,u,k]);return{form:c,apiKey:f,models:p,message:y,busy:g,isBuiltin:w,isEditing:S,setApiKey:m,update:k,submit:E,runTest:O,fetchModels:L,markIdEdited:()=>{x.current=!0}}}function Lc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=R(),c=$2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:f,models:m,message:p,busy:b,isBuiltin:y,isEditing:$}=c,g=y?u("llm.configureProvider",{name:e.name||e.id}):u($?"llm.editProvider":"llm.newProvider");return l`
    <${ei} open=${n} onClose=${r} title=${g} size="lg">
      <${ti} className="space-y-4">
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
            <${Gp} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Op.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${Io(e.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.baseUrl")}
          <${Tt} value=${d.baseUrl} placeholder=${e?.base_url||""} onChange=${v=>c.update("baseUrl",v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.apiKey")}
          <${Tt} type="password" value=${f} placeholder=${u("llm.apiKeyPlaceholder")} onChange=${v=>c.setApiKey(v.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${u("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Tt} value=${d.model} onChange=${v=>c.update("model",v.target.value)} />
            <${T} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${b!==""} onClick=${c.fetchModels}>
              ${u(b==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${m.length>0&&l`
          <${Gp} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
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
  `}function Pc({login:e}){let t=R(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
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
  `}function I4(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Uc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=Gs({settings:e,gatewayStatus:t}),[s,i]=h.default.useState(null),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(null),f=h.default.useRef(null),m=h.default.useCallback((g,v)=>{f.current&&window.clearTimeout(f.current),d({tone:g,text:v}),f.current=window.setTimeout(()=>d(null),3500)},[]);h.default.useEffect(()=>()=>{f.current&&window.clearTimeout(f.current)},[]);let p=h.default.useCallback((g=null)=>{i(g),u(!0)},[]),b=h.default.useCallback(async g=>{try{await r.setActiveProvider(g),m("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(p(g),m("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):m("error",v.message)}},[p,r,m,n]),y=h.default.useCallback(async({form:g,apiKey:v,provider:x})=>{if(x?.builtin){await r.saveBuiltinProvider({provider:x,form:g,apiKey:v}),m("success",n("llm.providerConfigured",{name:x.name||x.id}));return}let w=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:x});m("success",n(x?"llm.providerUpdated":"llm.providerAdded",{name:w.name||w.id}))},[r,m,n]),$=h.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),m("success",n("llm.providerDeleted"))}catch(v){m("error",v.message)}},[r,m,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>I4(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:p,closeDialog:()=>u(!1),handleUse:b,handleSave:y,handleDelete:$}}var K4=3e5;function H4(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function Q4(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function V4(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},K4);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var G4=3e5,Y4=9e5,J4=2e3;async function w2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,J4)),(await cc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function jc({onSuccess:e}={}){let t=R(),a=J(),[n,r]=h.default.useState(!1),[s,i]=h.default.useState(""),[o,u]=h.default.useState(!1),[c,d]=h.default.useState(""),[f,m]=h.default.useState(null),p=h.default.useCallback(()=>{i(""),d(""),m(null)},[]),b=h.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=h.default.useCallback(async v=>{if(p(),H4()){i(t("onboarding.nearaiLocalSso"));return}let x=window.open("about:blank","_blank");if(!x){i(t("onboarding.nearaiFailed"));return}try{x.opener=null}catch{}r(!0);try{let{auth_url:w}=await S$({provider:v,origin:window.location.origin});x.location.href=w;let S=await w2("nearai",G4,x);if(S==="active"){await b();return}x.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{x.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,p,t]),$=h.default.useCallback(async()=>{p(),r(!0);try{let v=Q4(),x=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!x){i(t("onboarding.nearaiFailed"));return}x.opener=null;let w=await V4(x,v);if(!w){i(t("onboarding.nearaiFailed"));return}await N$({account_id:w.accountId,public_key:w.publicKey,signature:w.signature,message:w.message,recipient:w.recipient,nonce:w.nonce}),await b()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,p,t]),g=h.default.useCallback(async()=>{p();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:x,verification_uri:w}=await _$();m({userCode:x,verificationUri:w}),v&&(v.location.href=w);let S=await w2("openai_codex",Y4,v);if(S==="active"){await b();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[b,p,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:f,startNearai:y,startNearaiWallet:$,startCodex:g}}var S2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",X4="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",Z4="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",W4="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",e5={nearai:{color:"#00ec97",path:X4},openai_codex:{color:"#10a37f",path:S2},openai:{color:"#10a37f",path:S2},anthropic:{color:"#d97757",path:Z4},ollama:{color:null,path:W4}};function N2({id:e,name:t}){let a=e5[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
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
  `}var t5=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function a5({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=h.default.useState(!1),o=h.default.useRef(null),u=t||a.nearaiBusy;h.default.useEffect(()=>{if(!s)return;let d=m=>{o.current&&!o.current.contains(m.target)&&i(!1)},f=m=>{m.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",f),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",f)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
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
  `}function n5({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${a5} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${T} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${T} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${ee} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${N2} id=${e.id} name=${u} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${u}</span>
            ${a&&l`<${F} tone="positive" label=${s("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${s(e.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${c}</div>
    <//>
  `}function _2(){let{isAdmin:e=!1,isChecking:t=!1}=Ba();return t?null:e?l`<${r5} />`:l`<${it} to="/chat" replace />`}function r5(){let e=R(),t=de(),a=J(),{gatewayStatus:n}=Ba(),r=Uc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=t5.map(f=>({entry:f,provider:s.providers.find(m=>m.id===f.id)})).filter(f=>f.provider),o=h.default.useCallback(()=>t("/chat"),[t]),u=jc({onSuccess:o}),c=h.default.useCallback(async f=>{let m=f.active_model||f.default_model||"";await Bo({provider_id:f.id,model:m}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=h.default.useCallback(async({form:f,apiKey:m,provider:p})=>{await r.handleSave({form:f,apiKey:m,provider:p});let b=p?.id||f.id.trim(),y=f.model?.trim()||p?.default_model||"";await Bo({provider_id:b,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
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
              <${n5}
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

        <${Pc} login=${u} />

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

      <${Lc}
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
  `}function z({children:e,className:t="",...a}){return l`<${ee} className=${t} ...${a}>${e}<//>`}function et({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
    <div
      className=${Q("px-1 py-4",s&&"border-t border-[var(--v2-panel-border)]",i)}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            ${e}
          </div>
          <div
            className=${Q("mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",o)}
          >
            ${t}
          </div>
          ${r&&l`<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${r}
          </div>`}
        </div>
        <${F} tone=${a} label=${n??a} />
      </div>
    </div>
  `}function k2({items:e}){return l`
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
  `}function ge({title:e,description:t,children:a,boxed:n=!0}){let r=l`
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
  `;return n?l`<${ee} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}var R2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Va({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",R2[e.type]||R2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var C2="",s5={workspace:"home"};function Fc(e){return s5[e]||e}function Jo(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function ri(e){return e?e.split("/").filter(Boolean):[]}function qc(e){return e?`/workspace/${ri(e).map(encodeURIComponent).join("/")}`:"/workspace"}function oh(e){let t=ri(e);return t.pop(),t.join("/")}function E2(e){return/\.mdx?$/i.test(e||"")}function zc({path:e,onNavigate:t}){let a=R(),n=ri(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,u=i===0?Fc(s):s;return l`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(qc(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${u}
          </button>
        `})}
    </div>
  `}function i5(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function T2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=R();if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(m=>!i5(m.path)),u=String(n||"").trim().toLowerCase(),c=u?o.filter(m=>m.name.toLowerCase().includes(u)):o,d=Jo(c),f;return o.length?d.length?f=l`
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
    <${z} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${zc} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${f}</div>
    <//>
  `}var Bc="/api/webchat/v2/fs",o5=1024*1024,l5=8*1024*1024;function A2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function u5(e,t){return t?`${e}/${t}`:e}function c5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function d5(e){return String(e||"").toLowerCase().startsWith("image/")}function m5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function f5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function p5(e,t){let a=new URL(`${Bc}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function h5(){return(await V(`${Bc}/mounts`))?.mounts||[]}async function si(e=""){if(!e)return{entries:(await h5()).map(o=>({name:Fc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=A2(e),n=new URL(`${Bc}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await V(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:u5(t,i.path),is_dir:i.kind==="directory"}))}}async function D2(e){let{mount:t,path:a}=A2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${Bc}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await V(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),u=p5(t,a),c={path:e,mime:i,size_bytes:o,download_path:u};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(d5(i)){if(o>l5)return{...c,kind:"binary"};let p=await sc(u);return{...c,kind:"image",image_data_url:p}}if(m5(i)||o>o5)return{...c,kind:"binary"};let d=await Na(u),f=new Uint8Array(await d.arrayBuffer());if(!c5(i)&&f5(f))return{...c,kind:"binary"};let m=new TextDecoder("utf-8").decode(f);return{...c,kind:"text",content:m}}function M2(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function v5(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!M2(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return Jo(r)}function O2({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=R(),u=n.has(e.path),c=B({queryKey:["workspace-list",e.path],queryFn:()=>si(e.path),enabled:e.is_dir&&u});if(e.is_dir){let d=v5(c.data?.entries,r,n);return l`
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
                  <${O2}
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
  `}function L2({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=R();if(i)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>l`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let u=Jo(e.filter(c=>!M2(c.path)));return u.length?l`
    <div className="space-y-1 p-2">
      ${u.map(c=>l`
        <${O2}
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
  `:l`<div className="px-4 py-8 text-sm text-iron-300">${o("workspace.noFiles")}</div>`}function P2({rootEntries:e,selectedPath:t,expandedPaths:a,filter:n,onFilterChange:r,isLoadingTree:s,onToggleDirectory:i,onSelectFile:o}){let u=R();return l`
    <${z} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${n}
          onInput=${c=>r(c.target.value)}
          placeholder=${u("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <${L2}
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
  `}function U2(e){return ri(e).pop()||"download"}function g5({path:e,file:t}){let a=R();return t.kind==="image"?l`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${U2(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?l`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${E2(e)?l`<${na} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:l`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function j2({path:e,file:t,isLoading:a,onNavigate:n}){let r=R(),[s,i]=h.default.useState(!1),o=h.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Na(t.download_path);kc(c,U2(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;if(!t||t.kind==="directory")return l`
      <${ge}
        title=${r("workspace.pickFileTitle")}
        description=${r("workspace.pickFileDesc")}
      />
    `;let u=r("workspace.fileMeta",{mime:t.mime||"application/octet-stream",size:Number(t.size_bytes||0)});return l`
    <${z} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${zc} path=${e} onNavigate=${n} />
        <div className="flex items-center gap-2">
          <${F} tone="muted" label=${u} />
          <${T}
            variant="secondary"
            size="sm"
            onClick=${o}
            disabled=${s}
          >${r("workspace.download")}<//>
        </div>
      </div>

      <${g5} path=${e} file=${t} />

      ${oh(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:oh(e)})}
        </div>
      `}
    <//>
  `}function F2(e){let t=R(),a=J(),[n,r]=h.default.useState(new Set),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=B({queryKey:["workspace-list",""],queryFn:()=>si("")}),d=B({queryKey:["workspace-file",e],queryFn:()=>D2(e),enabled:!!e}),f=e===""||d.data?.kind==="directory",m=B({queryKey:["workspace-list",e],queryFn:()=>si(e),enabled:f});h.default.useEffect(()=>{u(null)},[e]);let p=h.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>si(y)}),[a]),b=h.default.useCallback(async y=>{let $=new Set(n);if($.has(y)){$.delete(y),r($);return}$.add(y),r($);try{await p(y)}catch(g){u({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,p,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:f,currentEntries:m.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>u(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:m.isLoading,isFetching:c.isFetching||d.isFetching||m.isFetching,error:c.error||d.error||m.error||null,loadDirectory:p,toggleDirectory:b,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function lh(){let e=R(),t=de(),n=st()["*"]||C2,r=F2(n),s=h.default.useCallback(i=>{t(qc(i))},[t]);return l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${e("workspace.title")}</h1>
                <${F} tone="muted" label=${e("workspace.readOnly")} />
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
          <${Va}
            result=${r.result}
            onDismiss=${r.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${P2}
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
                  <${T2}
                    path=${n}
                    entries=${r.currentEntries}
                    isLoading=${r.isLoadingListing}
                    filter=${r.filter}
                    onOpen=${s}
                    onNavigate=${t}
                  />
                `:l`
                  <${j2}
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
  `}function q2(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function z2(){let t=((await Nx({limit:200}))?.projects||[]).map(q2);return{attention:[],projects:t}}async function B2(e){if(!e)return null;let t=await _x({projectId:e});return q2(t?.project)}function I2(e){return Promise.resolve({missions:[],todo:!0})}function K2(e){return Promise.resolve({threads:[],todo:!0})}function H2(e){return Promise.resolve({widgets:[],todo:!0})}function Q2(e){return Promise.resolve(null)}function V2(e){return Promise.resolve(null)}function G2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function Y2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function J2(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function X2(){let e=J(),t=B({queryKey:["projects-overview"],queryFn:z2,refetchInterval:5e3}),a=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function Z2(e){let t=J(),a=!!e,n=B({queryKey:["project-detail",e],queryFn:()=>B2(e),enabled:a,refetchInterval:a?7e3:!1}),r=B({queryKey:["project-missions",e],queryFn:()=>I2(e),enabled:a,refetchInterval:a?5e3:!1}),s=B({queryKey:["project-threads",e],queryFn:()=>K2(e),enabled:a,refetchInterval:a?4e3:!1}),i=B({queryKey:["project-widgets",e],queryFn:()=>H2(e),enabled:a,refetchInterval:a?15e3:!1}),o=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function W2({projectId:e,missionId:t,threadId:a}){let n=J(),[r,s]=h.default.useState(null),i=B({queryKey:["project-mission-detail",t],queryFn:()=>Q2(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=B({queryKey:["project-thread-detail",a],queryFn:()=>V2(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=H({mutationFn:({targetMissionId:m})=>G2(m),onSuccess:m=>{s({type:"success",message:m?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to fire mission"})}}),d=H({mutationFn:({targetMissionId:m})=>Y2(m),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to pause mission"})}}),f=H({mutationFn:({targetMissionId:m})=>J2(m),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending}}function Ic(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function Kc(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function eS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function tS(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function y5(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function aS(e){let t=y5(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function nS(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function Xo(e,t){return`${e} ${t}${e===1?"":"s"}`}var b5={projects:"muted",attention:"warning",spend:"success"};function rS({overview:e}){let t=nS(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:Kc(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${z} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${F} tone=${b5[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function x5(e){return e?.type==="failure"?"danger":"warning"}function $5(e){return e?.type==="failure"?"failure":"gate"}function sS({items:e,onOpenItem:t}){return e?.length?l`
    <${z} className="overflow-hidden border-amber-300/10 p-0">
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
              <${F} tone=${x5(a)} label=${$5(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function w5({project:e,onOpen:t,t:a}){return l`
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
        <${F} tone=${eS(e.health)} label=${e.health||"unknown"} />
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
            ${a("projects.card.threadsToday",{count:Xo(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${Xo(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:Xo(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:Kc(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${Ic(e.last_activity)}</div>
        </div>
        <${T}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function S5({project:e,onOpen:t,t:a}){return l`
    <${z}
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
            ${Xo(e.threads_today||0,"thread")} today
          </div>
          <${T}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function iS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=R(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?l`
      <${ge}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?l`
    <div className="space-y-5">
      ${u&&l`<${S5} project=${u} onOpen=${r} t=${o} />`}

      <${z} className="p-4 sm:p-5">
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
            ${c.map(d=>l`<${w5} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:l`
            <${ge}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${T} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
          `}
    </div>
  `:l`
      <${ge}
        title=${o("projects.empty.noneTitle")}
        description=${o("projects.empty.noneDesc")}
      >
        <${T} onClick=${s}>${o("projects.createFromChat")}<//>
      <//>
    `}function oS({threads:e,selectedThreadId:t,onSelectThread:a,onNewConversation:n,isStartingConversation:r}){let s=[...e].sort((i,o)=>new Date(o.updated_at||o.created_at)-new Date(i.updated_at||i.created_at));return l`
    <${z} className="p-4 sm:p-5">
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
        ${s.length?s.slice(0,18).map(i=>{let o=aS(i);return l`
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
                    <${F} tone=${tS(i.state)} label=${i.state} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                    <span>${i.step_count||0} steps</span>
                    <span>${i.total_tokens||0} tokens</span>
                    <span>${Ic(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var N5="/workspace";function _5(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function k5(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function lS({threadId:e}){let t=R(),[a,n]=h.default.useState(void 0),[r,s]=h.default.useState(null),i=B({queryKey:["project-files",e||"",a||""],queryFn:()=>gx({threadId:e,path:a}),enabled:!!e}),o=h.default.useMemo(()=>_5(i.data?.entries||[]),[i.data]),u=h.default.useCallback(async f=>{if(f.kind==="directory"){s(null),n(f.path);return}try{s(null);let m=await Na(rc({threadId:e,path:f.path})),p=URL.createObjectURL(m),b=document.createElement("a");b.href=p,b.download=f.name,document.body.appendChild(b),b.click(),b.remove(),URL.revokeObjectURL(p)}catch(m){s(m?.message||"Unable to download file")}},[e]),c=k5(a),d=l`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${F} tone="muted" label=${t("workspace.readOnly")} />
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
    <${z} className="p-4 sm:p-5">
      ${d}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${()=>n(void 0)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${c.map((f,m)=>{let p=`${N5}/${c.slice(0,m+1).join("/")}`;return l`
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
                  <${M}
                    name=${f.kind==="directory"?"folder":"file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${f.name}</span>
                  ${f.kind==="directory"?l`<${M} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`:l`<${M} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `):l`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `:l`
      <${z} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function R5(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function uS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=R5(t);return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
          ${e.description?l`<p className="mt-1 text-sm leading-6 text-iron-300">${e.description}</p>`:null}
        </div>

        <${oS}
          threads=${t}
          selectedThreadId=${a}
          onSelectThread=${n}
          onNewConversation=${r}
          isStartingConversation=${s}
        />
      </div>

      <${lS} threadId=${i} />
    </div>
  `}function Zo(){let e=R(),t=de(),{threadsState:a}=Ba(),{projectId:n=null,threadId:r=null}=st(),[s,i]=h.default.useState(""),[o,u]=h.default.useState(null),c=X2(),d=Z2(n),f=W2({projectId:n,threadId:r}),m=h.default.useMemo(()=>{let N=s.trim().toLowerCase();return N?c.overview.projects.filter(E=>[E.name,E.description,...E.goals||[]].some(O=>String(O||"").toLowerCase().includes(N))):c.overview.projects},[c.overview.projects,s]),p=h.default.useMemo(()=>c.overview.projects.find(N=>N.id===n)||null,[c.overview.projects,n]),b=h.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=h.default.useCallback(N=>{t(`/projects/${N}`)},[t]),$=h.default.useCallback(N=>{if(N.thread_id){t(`/projects/${N.project_id}/threads/${N.thread_id}`);return}t(`/projects/${N.project_id}`)},[t]),g=h.default.useCallback(async()=>{let N=null;u(null);try{N=await a.createThread()}catch(E){u({type:"error",message:E.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:N}})},[t,a]),v=h.default.useCallback(N=>{t(`/projects/${n}/threads/${N}`)},[t,n]),x=h.default.useCallback(async()=>{u(null);try{let N=await a.createThread(n);t("/chat",{state:{threadId:N}}),d.invalidate()}catch(N){u({type:"error",message:N.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),w=h.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=l`
    ${n&&l`<${T} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,k=null;return n?d.isLoading?k=l`
        <div className="space-y-4">
          ${[1,2,3].map(N=>l`<div key=${N} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!p?k=l`
        <${ge}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:k=l`
        <${uS}
          project=${d.project||p}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${x}
          isStartingConversation=${a.isCreating}
        />
      `:k=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(N=>l`<div key=${N} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${iS}
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
          <${Va} result=${o} onDismiss=${()=>u(null)} />
          <${Va} result=${f.actionResult} onDismiss=${f.clearActionResult} />
          ${!n&&l`
            <${rS} overview=${c.overview} />
            <${sS} items=${c.overview.attention} onOpenItem=${$} />
          `}
          ${k}
        </div>
      </div>
    </div>
  `}function Wo(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function el(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function cS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function dS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function Hc({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function C5({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=R();return e.status==="Active"?l`
      <${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.fireNow")}<//>
      <${T} variant="secondary" onClick=${()=>n(e.id)} disabled=${t}>${s("missions.action.pause")}<//>
    `:e.status==="Paused"?l`
      <${T} onClick=${()=>r(e.id)} disabled=${t}>${s("missions.action.resume")}<//>
      <${T} variant="secondary" onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runOnce")}<//>
    `:l`<${T} onClick=${()=>a(e.id)} disabled=${t}>${s("missions.action.runAgain")}<//>`}function mS({mission:e,isLoading:t,error:a,isBusy:n,onFire:r,onPause:s,onResume:i,onOpenProject:o,onOpenThread:u}){let c=R();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(d=>l`<div key=${d} className="v2-skeleton h-36 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${ge}
        title=${c("missions.unavailable")}
        description=${a?.message||c("missions.unavailableDesc")}
      />
    `:l`
    <div className="space-y-4">
      <${z} className="p-4 sm:p-5">
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
          <${F} tone=${el(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${Hc} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${Hc} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${Hc} label=${c("missions.meta.nextFire")} value=${Wo(e.next_fire_at)} />
          <${Hc} label=${c("missions.meta.updated")} value=${Wo(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${C5}
            mission=${e}
            isBusy=${n}
            onFire=${r}
            onPause=${s}
            onResume=${i}
          />
        </div>
      <//>

      <${z} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${na} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${z} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${na} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${z} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${na} content=${e.success_criteria} />
          </div>
        <//>
      `}

      ${e.threads?.length?l`
        <${z} className="p-4 sm:p-5">
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
                  <${F} tone=${el(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function E5(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function fS({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function T5({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=R(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${F} tone=${el(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:Wo(e.updated_at)})}
        </span>
        <${T}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function uh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:f}){let m=R(),p=E5(m);return l`
    <${z} className="p-4 sm:p-5">
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
        <${fS} value=${s} onChange=${i} label=${m("missions.filter.status")}>
          ${p.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}<//>`)}
        <//>
        <${fS} value=${o} onChange=${u} label=${m("missions.filter.project")}>
          <option value="all">${m("missions.filter.allProjects")}</option>
          ${c.map(b=>l`<option key=${b.id} value=${b.id}>${b.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(b=>l`
              <${T5}
                key=${b.id}
                mission=${b}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${f}
              />
            `):l`
              <${ge}
                title=${m("missions.emptyTitle")}
                description=${m("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function A5(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function pS({summary:e}){let t=R(),a=A5(t);return l`
    <${z} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${F} tone=${n.tone} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${e[n.key]||0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${n.key==="total"?t("missions.summary.completedFailed",{completed:e.completed||0,failed:e.failed||0}):t("missions.summary.acrossProjects")}
            </p>
          </div>
        `)}
      </div>
    <//>
  `}function hS(){return Promise.resolve({projects:[],todo:!0})}function vS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function gS(e){return Promise.resolve(null)}function yS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function bS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function xS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function $S(e){let t=B({queryKey:["mission-detail",e],queryFn:()=>gS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function D5(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function wS(){let e=J(),[t,a]=h.default.useState(null),n=B({queryKey:["projects-overview"],queryFn:hS,refetchInterval:7e3}),r=n.data?.projects||[],s=$d({queries:r.map(m=>({queryKey:["missions","project",m.id],queryFn:()=>vS({projectId:m.id}),refetchInterval:5e3,select:p=>p?.missions||[]}))}),i=s.flatMap((m,p)=>{let b=r[p];return(m.data||[]).map(y=>D5(y,b))}),o=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(m,p)=>({mutationFn:({missionId:b})=>m(b),onSuccess:()=>{a({type:"success",message:p}),o()},onError:b=>{a({type:"error",message:b.message||"Unable to update mission"})}}),c=H(u(yS,"Mission fired and a run was queued.")),d=H(u(bS,"Mission paused.")),f=H(u(xS,"Mission resumed."));return{projects:r,missions:i,summary:cS(i),isLoading:n.isLoading||s.some(m=>m.isLoading),isRefreshing:n.isFetching||s.some(m=>m.isFetching),error:n.error||s.find(m=>m.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending,invalidate:o}}function ch(){let e=R(),t=de(),{missionId:a=null}=st(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState("all"),c=wS(),d=$S(a),f=h.default.useMemo(()=>{let g=n.trim().toLowerCase();return dS(c.missions).filter(v=>{let x=!g||[v.name,v.goal,v.project?.name].some(k=>String(k||"").toLowerCase().includes(g)),w=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return x&&w&&S})},[c.missions,o,n,s]),m=h.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),p=d.mission?{...m,...d.mission,project:m?.project||null}:m,b=h.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=h.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),$=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${uh}
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
          <${mS}
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
        <${uh}
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

          <${Va}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${pS} summary=${c.summary} />

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
  `}var SS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],M5=new Set(["pending","in_progress"]),NS=new Set(["failed","interrupted","stuck","cancelled"]);function er(e){return e?String(e).replace(/_/g," "):"unknown"}function ii(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":NS.has(e)?"danger":"muted":"muted"}function O5(e){return M5.has(e)}function Qc(e){return O5(e?.state)}function _S(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":NS.has(e.state):!1}function Ur(e,t=8){return e?String(e).slice(0,t):"unknown"}function ra(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function kS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function dh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${ra(e.started_at)}`:null].filter(Boolean).join(" / ")}var L5=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function RS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function P5({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${t==="tool_use"?a.tool_name||"Tool call":a.tool_name||"Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${RS(t==="tool_use"?a.input:a.output||a.error||a)}</pre>
      </details>
    `:t==="message"?l`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${a.role||"assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${a.content||""}</div>
      </div>
    `:l`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t.replace(/_/g," ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${a.message||a.status||RS(a)}</div>
    </div>
  `}function CS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=R(),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(""),[c,d]=h.default.useState(!0),f=h.default.useRef(null),m=h.default.useMemo(()=>s==="all"?t:t.filter(b=>b.event_type===s),[t,s]);h.default.useEffect(()=>{c&&f.current&&(f.current.scrollTop=f.current.scrollHeight)},[c,m.length]);let p=h.default.useCallback(async(b=!1)=>{let y=o.trim();if(!(!y&&!b))try{await a({content:y||"(done)",done:b}),u("")}catch{}},[o,a]);return l`
    <${z} className="p-5 sm:p-6">
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
            ${L5.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}</option>`)}
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
                <${P5} event=${b} />
              </div>
            `):l`
              <${ge}
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
  `}function ES({job:e,activeTab:t,onTabChange:a,onBack:n,onCancel:r,onRestart:s,isBusy:i,children:o}){return l`
    <div className="space-y-5">
      <${z} className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick=${n} className="text-sm text-signal hover:text-white">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-semibold tracking-tight text-white">${e.title||"Untitled job"}</h2>
              <${F} tone=${ii(e.state)} label=${er(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Ur(e.id)}</span>
              <span>created ${ra(e.created_at)}</span>
              ${dh(e)&&l`<span>${dh(e)}</span>`}
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
            ${Qc(e)&&l`
              <${T} variant="secondary" disabled=${i} onClick=${()=>r(e.id)}>Cancel<//>
            `}
            ${_S(e)&&l`
              <${T} variant="primary" disabled=${i} onClick=${()=>s(e.id)}>Restart<//>
            `}
          </div>
        </div>
      <//>

      <div className="flex flex-wrap gap-2">
        ${SS.map(u=>l`
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
  `}function TS({nodes:e,depth:t=0,selectedPath:a,expandingPath:n,onToggleDirectory:r,onSelectPath:s}){return l`
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
        ${i.isDir&&i.expanded&&i.children?.length?l`<${TS}
              nodes=${i.children}
              depth=${t+1}
              selectedPath=${a}
              expandingPath=${n}
              onToggleDirectory=${r}
              onSelectPath=${s}
            />`:null}
      </div>
    `)}
  `}function AS({canBrowse:e,tree:t,selectedPath:a,selectedFile:n,fileError:r,isLoadingTree:s,isLoadingFile:i,expandingPath:o,treeError:u,onToggleDirectory:c,onSelectPath:d}){return e?l`
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <${z} className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          ${u&&l`<div className="mx-2 mb-3 rounded-md border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">${u}</div>`}
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
                  <${TS}
                    nodes=${t}
                    selectedPath=${a}
                    expandingPath=${o}
                    onToggleDirectory=${c}
                    onSelectPath=${d}
                  />
                `:l`<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>`}
        </div>
      <//>

      <${z} className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">${n?.path||a||"Select a file from the tree to inspect its contents."}</p>
        </div>

        ${r&&!i?l`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?l`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(f=>l`<div key=${f} className="v2-skeleton h-4 rounded" />`)}</div>`:n?l`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:l`
                <${ge}
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              `}
      <//>
    </div>
  `:l`
      <${ge}
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    `}function oi({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function DS({job:e}){let t=(e.transitions||[]).map(a=>({title:`${er(a.from)} -> ${er(a.to)}`,description:[ra(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${z} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${F} tone=${ii(e.state)} label=${er(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${oi} label="Created" value=${ra(e.created_at)} />
          <${oi} label="Started" value=${ra(e.started_at)} />
          <${oi} label="Completed" value=${ra(e.completed_at)} />
          <${oi} label="Duration" value=${kS(e.elapsed_secs)} />
          <${oi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${oi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${z} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${na} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${t.length?l`
              <${z} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${k2} items=${t} />
                </div>
              <//>
            `:l`
              <${ge}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `}function MS({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let f=R(),m=[{value:"all",label:f("jobs.list.filter.all")},{value:"pending",label:f("jobs.list.filter.pending")},{value:"in_progress",label:f("jobs.list.filter.inProgress")},{value:"completed",label:f("jobs.list.filter.completed")},{value:"failed",label:f("jobs.list.filter.failed")},{value:"stuck",label:f("jobs.list.filter.stuck")}];if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${ge}
        title=${f(t&&p?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${f(t&&p?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return l`
    <div className="space-y-5">
      <${z} className="p-4 sm:p-5">
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
                  <${F} tone=${ii(p.state)} label=${er(p.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Ur(p.id)}</span>
                  <span>${f("jobs.list.created",{value:ra(p.created_at)})}</span>
                  ${p.started_at&&l`<span>${f("jobs.list.started",{value:ra(p.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${Qc(p)&&l`
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
  `}var U5=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function OS({summary:e}){return l`
    <${z} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${U5.map(t=>l`
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
  `}function LS(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function PS(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function US(e){return Promise.resolve(null)}function jS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function FS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function qS(e){return Promise.resolve({events:[],todo:!0})}function zS(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function mh(e,t=""){return Promise.resolve({entries:[],todo:!0})}function BS(e,t){return Promise.resolve({content:"",todo:!0})}function IS(e){let t=J(),[a,n]=h.default.useState(null),r=B({queryKey:["job-detail",e],queryFn:()=>US(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=B({queryKey:["job-events",e],queryFn:()=>qS(e),enabled:!!e,refetchInterval:e?2500:!1}),i=H({mutationFn:({content:o,done:u})=>zS(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return h.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function KS(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function HS(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=HS(a.children,t);if(n)return n}}return null}function Vc(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:Vc(n.children,t,a)}:n)}function QS(e){let[t,a]=h.default.useState([]),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState(""),c=!!(e?.project_dir&&e?.id),d=B({queryKey:["job-files-root",e?.id],queryFn:()=>mh(e.id,""),enabled:c}),f=B({queryKey:["job-file",e?.id,n],queryFn:()=>BS(e.id,n),enabled:!!(c&&n)});h.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),h.default.useEffect(()=>{d.data?.entries?(a(KS(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let m=h.default.useCallback(async p=>{let b=HS(t,p);if(!(!b||!e?.id)){if(b.expanded){a(y=>Vc(y,p,$=>({...$,expanded:!1})));return}if(b.loaded){a(y=>Vc(y,p,$=>({...$,expanded:!0})));return}u(p);try{let y=await mh(e.id,p);a($=>Vc($,p,g=>({...g,expanded:!0,loaded:!0,children:KS(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:f.data||null,fileError:f.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:f.isLoading||f.isFetching,expandingPath:o,treeError:s,toggleDirectory:m}}function VS(){let e=J(),[t,a]=h.default.useState(null),n=B({queryKey:["jobs-summary"],queryFn:PS,refetchInterval:5e3}),r=B({queryKey:["jobs"],queryFn:LS,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=H({mutationFn:({jobId:u})=>jS(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${Ur(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=H({mutationFn:({jobId:u})=>FS(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${Ur(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function GS({result:e,onDismiss:t}){let a=R();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function fh(){let e=R(),t=de(),{jobId:a=null}=st(),[n,r]=h.default.useState(""),[s,i]=h.default.useState("all"),[o,u]=h.default.useState(a?"activity":"overview"),c=VS(),d=IS(a),f=QS(d.job);h.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let m=h.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(x=>{let w=!v||x.title.toLowerCase().includes(v)||x.id.toLowerCase().includes(v),S=s==="all"||x.state===s;return w&&S})},[c.jobs,n,s]),p=h.default.useCallback(v=>t(`/jobs/${v}`),[t]),b=h.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=h.default.useCallback(async v=>{try{let x=await c.restartJob({jobId:v});x?.new_job_id&&t(`/jobs/${x.new_job_id}`)}catch{}},[c,t]),$=l`
    ${a&&l`<${T} variant="ghost" onClick=${()=>t("/jobs")}
      >${e("jobs.allJobs")}<//
    >`}
  `,g=null;if(a)if(d.isLoading)g=l`
        <div className="space-y-4">
          ${[1,2,3].map(v=>l`<div key=${v} className="v2-skeleton h-32 rounded-[18px]" />`)}
        </div>
      `;else if(d.error||!d.job)g=l`
        <${ge}
          title=${e("jobs.unavailable")}
          description=${d.error?.message||e("jobs.unavailableDesc")}
        >
          <${T} variant="secondary" onClick=${()=>t("/jobs")}
            >${e("jobs.returnToJobs")}<//
          >
        <//>
      `;else{let v={overview:l`<${DS} job=${d.job} />`,activity:l`
          <${CS}
            job=${d.job}
            events=${d.events}
            onSendPrompt=${d.sendPrompt}
            isSendingPrompt=${d.isSendingPrompt}
          />
        `,files:l`
          <${AS}
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
        <${ES}
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
          <${MS}
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
          <${GS}
            result=${c.actionResult}
            onDismiss=${c.clearActionResult}
          />
          <${GS}
            result=${d.promptResult}
            onDismiss=${d.clearPromptResult}
          />
          <${OS} summary=${c.summary} />
          ${g}
        </div>
      </div>
    </div>
  `}function tr(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function Gc(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function Yc(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function YS(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function JS(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function j5(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function XS({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${F} tone=${j5(t.status)} label=${t.status} />
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
  `}function ZS({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function WS({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=de(),u=R();return t?l`
      <div className="space-y-4">
        ${[1,2,3].map(c=>l`<div key=${c} className="v2-skeleton h-32 rounded-xl" />`)}
      </div>
    `:a||!e?l`
      <${ge}
        title=${u("routine.unavailable")}
        description=${a?.message||u("routine.unavailableDesc")}
      />
    `:l`
    <${z} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              ${e.name}
            </h2>
            <${F}
              tone=${Gc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${F}
              tone=${Yc(e.verification_status)}
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
        <${ar} label="Action" value=${JS(e.action)} />
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
        <${ZS} title=${u("routine.triggerPayload")} value=${e.trigger} />
        <${ZS} title=${u("routine.actionPayload")} value=${e.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <${XS} runs=${e.recent_runs} />
      </div>
    <//>
  `}function eN({routine:e,selectedRoutineId:t,onSelectRoutine:a,onTriggerRoutine:n,onToggleRoutine:r,isBusy:s}){let i=t===e.id;return l`
    <article
      className=${["group flex flex-col gap-4 rounded-[18px] border p-5",i?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80"].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick=${()=>a(e.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">${e.name}</h3>
            <${F}
              tone=${Gc(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${F}
              tone=${Yc(e.verification_status)}
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
  `}var F5=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function ph({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:f}){let m=R();if(!e.length){let p=!!n.trim()||s!=="all";return l`
      <${ge}
        title=${t&&p?"No routines match":"No routines yet"}
        description=${t&&p?"Adjust the search or status filter to find a saved routine.":"Routines created from chat will appear here after they are saved."}
      />
    `}return l`
    <div className="space-y-5">
      <${z} className="p-4 sm:p-5">
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
            ${F5.map(p=>l`<option key=${p.value} value=${p.value}>${p.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${e.map(p=>l`
            <${eN}
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
  `}var q5=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function tN({summary:e}){return l`
    <${z} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${q5.map(t=>l`
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
  `}function aN(e){let[t,a]=h.default.useState(""),[n,r]=h.default.useState("all");return{filteredRoutines:h.default.useMemo(()=>{let i=t.trim().toLowerCase();return YS(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function nN(){return Promise.resolve({routines:[],todo:!0})}function rN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function sN(e){return Promise.resolve(null)}function Jc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function Xc(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function iN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function oN(e){let t=J(),[a,n]=h.default.useState(null),r=B({queryKey:["routine-detail",e],queryFn:()=>sN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:f=>{n({type:"error",message:f.message||"Unable to update routine"})}}),o=H(i(Jc,"Routine run queued.")),u=H(i(Xc,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function lN(){let e=J(),[t,a]=h.default.useState(null),n=B({queryKey:["routines-summary"],queryFn:rN,refetchInterval:5e3}),r=B({queryKey:["routines"],queryFn:nN,refetchInterval:5e3}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,f)=>({mutationFn:({routineId:m})=>d(m),onSuccess:()=>{a({type:"success",message:f}),s()},onError:m=>{a({type:"error",message:m.message||"Unable to update routine"})}}),o=H(i(Jc,"Routine run queued.")),u=H(i(Xc,"Routine status updated.")),c=H(i(iN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function hh(){let e=de(),{routineId:t=null}=st(),a=lN(),n=oN(t),r=aN(a.routines),s=h.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=h.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${ph}
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
          <${WS}
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
        <${ph}
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

          <${Va}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Va}
            result=${n.actionResult}
            onDismiss=${n.clearActionResult}
          />
          <${tN} summary=${a.summary} />

          ${a.isLoading?l`
                <div className="space-y-4">
                  ${[1,2,3].map(u=>l`<div key=${u} className="v2-skeleton h-32 rounded-xl" />`)}
                </div>
              `:o}
        </div>
      </div>
    </div>
  `}function z5(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function B5(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function uN({deliveryState:e}){let t=R(),a=e.currentTarget?.target_id||"",[n,r]=h.default.useState(a),[s,i]=h.default.useState(!1),o=h.default.useRef(null);h.default.useEffect(()=>{r(a)},[a]),h.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,f=!!a&&!c,m=e.finalReplyTargets.length>0,p=e.targets.some(O=>O?.capabilities?.final_replies&&O?.target?.status==="unavailable"),b=m||p,y=O=>(o.current&&clearTimeout(o.current),i(!1),O.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),$=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{f&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),x=e.currentStatus,w=x==="available"?"success":x==="unavailable"?"warning":"muted",S=t(x==="available"?"automations.delivery.pill.ready":x==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),k=!!e.currentTarget,N=t(k?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),E=B5(t("automations.delivery.footnote"),{command:l`<code
        key="cmd"
        className="rounded px-1.5 py-0.5 font-mono text-[0.6875rem] bg-[var(--v2-surface-muted)] text-[var(--v2-accent-text)]"
      >
        approve &lt;code&gt;
      </code>`});return l`
    <${z} className="p-5 sm:p-6">
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
        ${k&&l`
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
              <${F} tone=${w} label=${S} />
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
            ${e.finalReplyTargets.map(O=>{let L=O?.target?.target_id??"",U=O?.target?.display_name||O?.target?.target_id||"",D=O?.target?.description||"",K=O?.target?.status??"available",te=n===L;return l`
                <label
                  key=${L}
                  className=${Q("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",te&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${L}
                    checked=${te}
                    disabled=${c}
                    onChange=${()=>r(L)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${U}
                    </div>
                    ${D&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${D}
                    </div>`}
                  </div>
                  <${F}
                    tone=${z5(K)}
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
                <${F}
                  tone="warning"
                  label=${t("automations.delivery.pill.notPaired")}
                  className="shrink-0"
                />
              </div>
            `}

            <!-- Web app only / fallback row -->
            <label
              className=${Q("flex items-start gap-3.5 rounded-xl border px-4 py-3.5","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]",m?"cursor-pointer hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]":"cursor-default",n===""&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
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
              <${F}
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
            <${M} name="check" className="h-3.5 w-3.5" />
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
        ${b&&l`
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            ${E}
          </div>
        `}

      </div>
    <//>
  `}var I5=["schedule","once"],dN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},mN={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},fN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function sa(e){return typeof e=="function"?e:t=>t}var gh=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:xn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:rD},{value:"completed",labelKey:"automations.filter.completed",predicate:sD}];function pN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>I5.includes(r?.source?.type)).map(r=>W5(r,t,a)).sort(nD)}function hN(e,t){let a=gh.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function vN(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>xn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>xn(i)&&vh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function K5(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=uD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:f}=s,m=t&&typeof t=="string"?t:null,p=m?` (${m})`:"",b=f==="*"&&u==="*"&&c==="*"&&d==="*";if(b&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=cD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(nr(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=iD(o,i,n);if(!y)return r("automations.schedule.custom");if(b)return r("automations.schedule.everyDayAt",{time:y})+p;let $=dD(d);if(f==="*"&&u==="*"&&c==="*"&&$==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+p;if(f==="*"&&u==="*"&&c==="*"&&nr($,0,7)){let g=oD(Number($)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+p}if(f==="*"&&nr(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:y})+p;if(nr(u,1,31)&&nr(c,1,12)&&d==="*"&&(f==="*"||nr(f,1970,9999))){let g=lD(Number(c),Number(u),f==="*"?null:Number(f),n);return r("automations.schedule.dateAt",{date:g,time:y})+p}return r("automations.schedule.custom")}function jr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function gN(e,t){let a=dN[e]?.labelKey||"automations.state.unknown";return sa(t)(a)}function yN(e){return dN[e]?.tone||"muted"}function H5(e,t){return xn(e)&&e?.has_running_run?sa(t)("automations.status.running"):xn(e)&&e?.has_failed_runs?sa(t)("automations.status.needsReview"):gN(e?.state,t)}function Q5(e){return xn(e)&&e?.has_running_run?"info":xn(e)&&e?.has_failed_runs?"danger":yN(e?.state)}function V5(e,t){let a=mN[e]?.labelKey||"automations.lastStatus.none";return sa(t)(a)}function G5(e){return mN[e]?.tone||"muted"}function Y5(e,t){let a=fN[Zc(e)]?.labelKey||"automations.runStatus.unknown";return sa(t)(a)}function J5(e){return fN[Zc(e)]?.tone||"muted"}function X5(e,t,a,n){if(!e)return sa(a)("automations.schedule.custom");let r=jr(e,null,n,t);if(!r)return sa(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return sa(a)("automations.schedule.onceAt",{datetime:r})+s}function Z5(e,t,a){return e?.type==="once"?X5(e.at,e.timezone,t,a):e?.type==="schedule"?K5(e.cron,e.timezone||"UTC",t,a):sa(t)("automations.schedule.custom")}function W5(e,t,a){let n=sa(t),r=eD(e.recent_runs,t,a),s=r[0]||null,i=r.find(f=>f.status==="running")||null,o=r.find(f=>f.status==="ok"||f.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(f=>f.status==="running"),has_failed_runs:r.some(f=>f.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:Z5(e.source,t,a),state_label:gN(e.state,t),state_tone:yN(e.state),primary_status_label:H5(d,t),primary_status_tone:Q5(d),next_run_timestamp:yh(e.next_run_at),next_run_label:jr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:jr(c,n("automations.date.noRuns"),a),last_status_label:V5(u,t),last_status_tone:G5(u),created_label:jr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:aD(r,t)}}function eD(e,t,a){let n=sa(t);return Array.isArray(e)?e.map(r=>{let s=Zc(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=yh(i);return{...r,status:s,status_label:Y5(s,t),status_tone:J5(s),timestamp:o,timestamp_source:i,fired_label:jr(i,n("automations.date.unscheduled"),a),submitted_label:jr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:jr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function Zc(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function bN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t)a[Zc(n?.status)]+=1;return a}function tD(e){let t=bN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function xN(e,t){let a=sa(t),n=bN(e),r=tD(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function aD(e,t){let a=sa(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function nD(e,t){let a=xn(e),n=xn(t);return a!==n?a?-1:1:(vh(e)??Number.MAX_SAFE_INTEGER)-(vh(t)??Number.MAX_SAFE_INTEGER)}function yh(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function xn(e){return e?.state==="active"||e?.state==="scheduled"}function rD(e){return["paused","disabled","inactive"].includes(e?.state)}function sD(e){return e?.state==="completed"}function vh(e){return e?.next_run_timestamp??yh(e?.next_run_at)}function bh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function iD(e,t,a){return!nr(e,0,23)||!nr(t,0,59)?null:bh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function oD(e,t){return bh(t,{weekday:"long"},new Date(2001,0,7+e))}function lD(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return bh(n,r,new Date(a??2e3,e-1,t))}function uD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&cN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&cN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function cN(e){return/^0+$/.test(e)}function nr(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function cD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function dD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}function mD(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function $N({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function wN(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(mD),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var fD=8;function xh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function Wc({runs:e=[]}){let t=R(),a=e.slice(0,fD);if(!a.length)return l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let n=e.length-a.length;return l`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:a.length,total:e.length})}
    >
      ${a.map(r=>l`
        <span
          key=${xh(r)}
          title=${`${r.status_label} \xB7 ${r.fired_label}`}
          className=${Q("h-3 w-3 rounded-full border",r.status==="ok"&&"border-emerald-300/50 bg-emerald-400",r.status==="error"&&"border-red-300/50 bg-red-400",r.status==="running"&&"border-sky-300/60 bg-sky-400",r.status==="unknown"&&"border-iron-500 bg-iron-600")}
        />
      `)}
      ${n>0&&l`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf",{shown:a.length,total:e.length})}
      >
        +${n}
      </span>`}
    </div>
  `}function ed({runs:e=[],className:t=""}){let a=R(),n=xN(e,a);return n.total?l`
    <div className=${Q("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>l`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:l`<span className=${Q("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function SN({run:e,onOpenRun:t,onOpenLogs:a}){let n=R(),r=!!e.chat_path,s=$N({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${F} tone=${e.status_tone} label=${e.status_label} />
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
          <${M} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${T}
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
  `}function td({label:e,value:t,tone:a}){return l`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${e}
      </div>
      <div
        className=${Q("mt-2 min-w-0 break-words text-sm text-iron-100",a==="success"&&"text-emerald-200",a==="danger"&&"text-red-200",a==="info"&&"text-sky-200")}
      >
        ${t||"\u2014"}
      </div>
    </div>
  `}function NN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=R(),i=de();if(!e)return l`
      <${z} className="p-4 sm:p-5">
        <${ge}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,u=e.state==="paused",c=e.state==="active"||e.state==="scheduled",f=`${s(u?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,m=()=>{if(u){n?.(e.automation_id);return}c&&a?.(e.automation_id)},p=`${s("common.delete")}: ${e.display_name}`,b=()=>{window.confirm(p)&&r?.(e.automation_id)};return l`
    <${z} className="overflow-hidden">
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
            <${F}
              tone=${e.primary_status_tone}
              label=${e.primary_status_label}
            />
            ${(c||u)&&l`
              <${T}
                type="button"
                variant=${u?"primary":"secondary"}
                size="icon-sm"
                aria-label=${f}
                title=${f}
                disabled=${t}
                onClick=${m}
              >
                <${M} name=${u?"play":"pause"} className="h-4 w-4" />
              <//>
            `}
            <${T}
              type="button"
              variant="danger"
              size="icon-sm"
              aria-label=${p}
              title=${p}
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
          <${td} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${td}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${td} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${td}
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
              <${Wc} runs=${e.recent_runs} />
              <${ed} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(y=>l`
                    <${SN}
                      key=${xh(y)}
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
  `}var pD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function hD({promptKey:e}){let t=R(),a=t(e),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>()=>clearTimeout(s.current),[]),l`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${a}</span>
      <button
        type="button"
        onClick=${async()=>{try{await navigator.clipboard.writeText(a),r(!0),clearTimeout(s.current),s.current=setTimeout(()=>r(!1),1500)}catch{}}}
        aria-label=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        title=${t(n?"automations.empty.copied":"automations.empty.copyPrompt")}
        className=${Q("inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",n&&"text-emerald-300")}
      >
        <${M} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function _N(){let e=R(),t=de();return l`
    <${z} className="p-6 sm:p-8">
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
            ${pD.map(a=>l`<${hD} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${T} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${M} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function kN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:u,onResumeAutomation:c,onDeleteAutomation:d}){let f=R(),m=hN(e,t),p=e.length>0,b=m.find(y=>y.automation_id===i)||m[0]||null;return l`
    <div className="space-y-5">
      <${z} className="p-4 sm:p-5">
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
              className="inline-flex max-w-full overflow-x-auto rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label=${f("automations.filterLabel")}
            >
              ${gh.map(y=>l`
                <button
                  key=${y.value}
                  type="button"
                  aria-pressed=${t===y.value}
                  onClick=${()=>a(y.value)}
                  className=${Q("min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${f(y.labelKey)}
                </button>
              `)}
            </div>
            <${T}
              variant="secondary"
              size="icon-sm"
              aria-label=${f("automations.refresh")}
              title=${f(r?"automations.refreshing":"automations.refresh")}
              disabled=${r}
              onClick=${n}
            >
              <${M}
                name="retry"
                className=${Q("h-4 w-4",r&&"v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${m.length?l`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${z} className="overflow-hidden">
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
                            className=${Q("border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",$&&"bg-[var(--v2-accent-soft)]/30")}
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
                                <${Wc} runs=${y.recent_runs} />
                                <${ed} runs=${y.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${F}
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

              <${NN}
                automation=${b}
                isMutating=${s}
                onPauseAutomation=${u}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:p?l`
              <${ge}
                title=${f("automations.empty.matchingTitle")}
                description=${f("automations.empty.matchingDescription")}
              />
            `:l`<${_N} />`}
    </div>
  `}function RN({summary:e,activeFilter:t,onSelectFilter:a}){let n=R(),r=[{key:"scheduled",label:n("automations.summary.scheduled"),value:e?.scheduled??0,tone:"muted",detail:n("automations.summary.scheduledDetail"),filter:"all"},{key:"active",label:n("automations.summary.active"),value:e?.active??0,tone:"signal",detail:n("automations.summary.activeDetail"),filter:"active"},{key:"running",label:n("automations.summary.running"),value:e?.running??0,tone:"info",detail:n("automations.summary.runningDetail"),filter:"running"},{key:"failures",label:n("automations.summary.failures"),value:e?.failures??0,tone:(e?.failures??0)>0?"danger":"success",detail:n("automations.summary.failuresDetail"),filter:(e?.failures??0)>0?"failures":null},{key:"nextRun",label:n("automations.summary.nextRun"),value:e?.nextRun||n("automations.summary.none"),tone:"info",detail:n("automations.summary.nextRunDetail"),valueClassName:"text-lg md:text-xl"}];return l`
    <${z} className="p-4 sm:p-5">
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
              className=${Q(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${u}
            </button>
          `:l`<div key=${s.key} className=${c}>${u}</div>`})}
      </div>
    <//>
  `}function vD(e){return e==="active"||e==="scheduled"}function gD(e){return Number.isFinite(e)?e:null}function CN(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!vD(r.state)))continue;let s=gD(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var bD=50,xD=25;function EN(e=!1){let{t,lang:a}=rl(),n=J(),r=B({queryKey:["automations",{includeCompleted:e}],queryFn:()=>bx({limit:bD,runLimit:xD,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=h.default.useMemo(()=>pN(r.data,t,a),[r.data,t,a]),i=h.default.useMemo(()=>vN(s),[s]),o=h.default.useMemo(()=>CN(s),[s]);h.default.useEffect(()=>{if(o==null)return;let p=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(p)},[o,r.refetch]);let u=r.data?.scheduler_enabled!==!1,c=h.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=H({mutationFn:p=>xx({automationId:p}),onSuccess:c}),f=H({mutationFn:p=>$x({automationId:p}),onSuccess:c}),m=H({mutationFn:p=>wx({automationId:p}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:u,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||f.isPending||m.isPending,error:r.error||null,actionError:d.error||f.error||m.error||null,pauseAutomation:d.mutate,resumeAutomation:f.mutate,deleteAutomation:m.mutate,refetch:r.refetch}}var TN=["outbound-delivery","preferences"],AN=["outbound-delivery","targets"];function DN(){let e=J(),t=B({queryKey:TN,queryFn:kx}),a=B({queryKey:AN,queryFn:Rx}),n=H({mutationFn:({finalReplyTargetId:i})=>Cx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(TN,i),e.invalidateQueries({queryKey:AN})}}),r=h.default.useMemo(()=>a.data?.targets??[],[a.data]),s=h.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function MN(){let e=R(),[t,a]=h.default.useState("all"),[n,r]=h.default.useState(null),i=EN(t==="completed"),o=DN(),[u,c]=h.default.useState(!1),d=h.default.useRef(null);h.default.useEffect(()=>()=>clearTimeout(d.current),[]);let f=h.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),m=i.isRefreshing||u,p=i.error&&!i.isLoading&&i.automations.length===0;return h.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),l`
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
                <${RN}
                  summary=${i.summary}
                  activeFilter=${t}
                  onSelectFilter=${a}
                />
                <${uN} deliveryState=${o} />

                ${i.isLoading?l`
                      <div className="space-y-4">
                        ${[1,2,3].map(b=>l`<div
                              key=${b}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${kN}
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
  `}var ON={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function LN({result:e,onDismiss:t}){return h.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",ON[e.type]||ON.info].join(" ")}>
      <${M}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${M} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var UN="/api/webchat/v2/channels/slack/setup";function jN(){return V(UN)}function FN(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:PN(e.user_id),shared_subject_user_id:PN(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),V(UN,{method:"PUT",body:JSON.stringify(t)})}function $h(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function PN(e){let t=String(e||"").trim();return t||null}var qN="/api/webchat/v2/channels/slack/allowed",$D="/api/webchat/v2/channels/slack/subjects";function zN(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function BN(){return V(qN)}function IN(){return V($D)}function KN(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return V(qN,{method:"PUT",body:JSON.stringify(n)})}function HN(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var QN=["slack-allowed-channels"];function GN({action:e}){let t=R(),a=J(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState([]),c=SD(e,t),d=B({queryKey:QN,queryFn:BN}),f=B({queryKey:["slack-routable-subjects"],queryFn:IN}),m=f.data?.subjects||[],p=VN(m),b=f.isSuccess||f.isError,y=m.length>0;h.default.useEffect(()=>{d.data&&u(wh(d.data.channels||[]))},[d.data]);let $=H({mutationFn:({channels:k})=>KN(k),onSuccess:k=>{u(wh(k.channels||[])),a.invalidateQueries({queryKey:QN}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let k=n.trim();!k||!f.isSuccess||(u(N=>wh([...N,{channel_id:k,subject_user_id:s}])),r(""))},v=k=>{u(N=>N.filter(E=>E.channel_id!==k))},x=(k,N)=>{u(E=>E.map(O=>O.channel_id===k?{...O,subject_user_id:N}:O))},w=()=>{$.mutate({channels:wD(o)})},S=f.isError&&o.some(k=>!k.subject_user_id);return l`
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
          ${!y&&l`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&l`<option value="">${c.autoSubjectLabel}</option>`}
          ${p.map(k=>l`
              <option key=${k.subject_user_id} value=${k.subject_user_id}>
                ${k.display_name}
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
        ${o.map(k=>l`
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
                ${y?l`
                    <select
                      value=${k.subject_user_id}
                      onChange=${N=>x(k.channel_id,N.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${VN(m,k).map(N=>l`
                          <option key=${N.subject_user_id} value=${N.subject_user_id}>
                            ${N.display_name}
                          </option>
                        `)}
                    </select>
                  `:l`<span className="max-w-40 truncate text-xs text-iron-500">
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
          ${HN($.error||d.error||f.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function VN(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function wh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return zN(Array.from(t.keys())).map(a=>t.get(a))}function wD(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function SD(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Sh=["slack-setup"],Fr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function XN({action:e}){let t=B({queryKey:Sh,queryFn:jN}),a=t.data?.configured===!0;return l`
    <div className="space-y-3">
      <${ND} action=${e} setupQuery=${t} />
      ${a&&l`<${GN} action=${e} />`}
    </div>
  `}function ND({action:e,setupQuery:t}){let a=J(),[n,r]=h.default.useState(_D()),s=h.default.useRef(!1),i=h.default.useRef(!1),o=t.data,u=kD(e);h.default.useEffect(()=>{!o||s.current||i.current||(r(YN(o)),s.current=!0)},[o]);let c=H({mutationFn:FN,onSuccess:p=>{i.current=!1,r(YN(p)),s.current=!0,a.setQueryData(Sh,p),a.invalidateQueries({queryKey:Sh}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=p=>b=>{i.current=!0,r(y=>({...y,[p]:b.target.value}))},f=()=>c.mutate(n),m=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return l`
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
        ${tl("Installation ID",n.installation_id,d("installation_id"),"",Fr.installationId)}
        ${tl("Team ID",n.team_id,d("team_id"),"",Fr.teamId)}
        ${tl("App ID",n.api_app_id,d("api_app_id"),"",Fr.appId)}
        ${tl("Bot user",n.user_id,d("user_id"),"default operator",Fr.botUser)}
        ${tl("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Fr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${JN("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Fr.botToken)}
        ${JN("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Fr.signingSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${T}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${f}
          disabled=${!m||c.isPending}
        >
          ${c.isPending?"Saving...":u.submitLabel}
        <//>
        ${t.isError&&l`<p className="text-xs text-red-300">
          ${$h(t.error,u.errorMessage)}
        </p>`}
        ${c.isError&&l`<p className="text-xs text-red-300">
          ${$h(c.error,u.errorMessage)}
        </p>`}
        ${c.isSuccess&&l`<p className="text-xs text-emerald-300">${u.successMessage}</p>`}
      </div>
    </div>
  `}function YN(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function _D(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function tl(e,t,a,n="",r=null){return l`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${e}</span>
      <input
        type="text"
        value=${t}
        onChange=${a}
        placeholder=${n}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${ZN} help=${r} />
    </label>
  `}function JN(e,t,a,n,r=null){return l`
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
      <${ZN} help=${r} />
    </label>
  `}function ZN({help:e}){return e?l`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${e.body}</span>
      ${e.example&&l`<span className="mt-0.5 block font-mono text-iron-300">${e.example}</span>`}
    </p>
  `:null}function kD(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Nh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function qr(e){return e==="wasm_channel"||e==="channel"}var WN={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},e_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function t_(e){let t=a_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||qr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function a_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function _h(e){let t=a_(e);return t==="active"||t==="ready"}function n_({extension:e,secrets:t=[],fields:a=[]}={}){return _h(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var r_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",s_="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",i_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",o_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",l_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",RD="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function u_(e){return e.package_ref?.id||""}function CD({actions:e,isBusy:t}){let a=R(),[n,r]=h.default.useState(!1),s=h.default.useRef(null);return h.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
  `}function c_({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${RD}>${t}</span>`)}
    </div>
  `}function li({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=R(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=WN[i]||"muted",u=s(`extensions.state.${i}`)||e_[i]||i,c=s(`extensions.kind.${e.kind}`)||Nh[e.kind]||e.kind,d=e.display_name||u_(e),f=!!e.package_ref,m=e.tools||[],[p,b]=h.default.useState(!1),$=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],x=[],w=t_(e);w==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):w==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),f&&(e.needs_setup||e.has_auth)&&w!=="configure"&&x.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),f&&qr(e.kind)&&(i==="setup_required"||i==="failed")&&x.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),f&&qr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&x.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),f&&x.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return l`
    <div className=${r_}>
      <div className="flex items-start gap-2">
        <${F} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${x.length>0&&l`<${CD} actions=${x} isBusy=${r} />`}
      </div>

      <div className=${s_}>
        <span>${c}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${i_}>${e.description}</p>`}

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

      <div className=${o_}>
        ${m.length>0?l`
              <button
                type="button"
                aria-expanded=${p?"true":"false"}
                onClick=${()=>b(k=>!k)}
                className=${l_}
              >
                <${M} name="layers" className="h-3.5 w-3.5" />
                <span>${m.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:m.length})}</span>
                <${M}
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

      ${p&&l`<${c_} items=${m} />`}
    </div>
  `}function zr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=R(),s=r(`extensions.kind.${e.kind}`)||Nh[e.kind]||e.kind,i=e.display_name||u_(e),o=!!(e.package_ref&&t),u=e.keywords||[],[c,d]=h.default.useState(!1);return l`
    <div className=${r_}>
      <div className="flex items-start gap-2">
        <${F}
          tone="muted"
          label=${n||r("extensions.state.available")||"available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${i}
        </span>
      </div>

      <div className=${s_}>
        <span>${s}</span>
        ${e.version&&l`<span>· v${e.version}</span>`}
      </div>

      ${e.description&&l`<p className=${i_}>${e.description}</p>`}

      <div className=${o_}>
        ${u.length>0?l`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(f=>!f)}
                className=${l_}
              >
                <${M} name="list" className="h-3.5 w-3.5" />
                <span>${u.length===1?r("extensions.oneKeyword"):r("extensions.pluralKeywords",{count:u.length})}</span>
                <${M}
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
            <${M} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        `}
      </div>

      ${c&&l`<${c_} items=${u} />`}
    </div>
  `}function d_(){return V("/api/webchat/v2/extensions")}function m_(){return V("/api/webchat/v2/extensions/registry")}function f_(e){return V("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function p_(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(al(e))}/activate`,{method:"POST"})}function h_(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(al(e))}/remove`,{method:"POST"})}function v_(e){return V(`/api/webchat/v2/extensions/${encodeURIComponent(al(e))}/setup`)}function g_(e,t,a){return Px(al(e),{action:"submit",payload:{secrets:t,fields:a}})}function y_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return V(`/api/webchat/v2/extensions/${encodeURIComponent(al(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function b_(){return Promise.resolve({requests:[]})}function x_(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function al(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var ED=2e3,TD=10*60*1e3;function ui(e){return e?.package_ref?.id||null}function kh(e){return e?.display_name||ui(e)||""}function $_(e,t,a){return ui(t)||`${e}:${kh(t)||"unknown"}:${a}`}function AD(e,t){return e.installed!==t.installed?e.installed?-1:1:kh(e.entry||e.extension).localeCompare(kh(t.entry||t.extension))}function w_(){let e=J(),t=B({queryKey:["gateway-status-extensions"],queryFn:Hs,staleTime:1e4}),a=B({queryKey:["extensions"],queryFn:d_}),n=B({queryKey:["extension-registry"],queryFn:m_}),r=B({queryKey:["connectable-channels"],queryFn:Tc}),s=h.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=h.default.useState(null),u=h.default.useCallback(()=>o(null),[]),c=H({mutationFn:({packageRef:D})=>f_(D),onSuccess:(D,{displayName:K})=>{D.success?(o({type:"success",message:D.message||D.instructions||`${K||"Extension"} installed`}),D.auth_url&&window.open(D.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:D.message||"Install failed"}),s()},onError:D=>{o({type:"error",message:D.message}),s()}}),d=H({mutationFn:({packageRef:D})=>p_(D),onSuccess:(D,{displayName:K})=>{D.success?(o({type:"success",message:D.message||D.instructions||`${K||"Extension"} activated`}),D.auth_url&&window.open(D.auth_url,"_blank","noopener,noreferrer")):D.auth_url?(window.open(D.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):D.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:D.message||"Activation failed"}),s()},onError:D=>{o({type:"error",message:D.message})}}),f=H({mutationFn:({packageRef:D})=>h_(D),onSuccess:(D,{displayName:K})=>{D.success?o({type:"success",message:`${K||"Extension"} removed`}):o({type:"error",message:D.message||"Remove failed"}),s()},onError:D=>{o({type:"error",message:D.message})}}),m=t.data||{},p=a.data?.extensions||[],b=n.data?.entries||[],y=r.data?.channels||[],$=new Map(p.map(D=>[ui(D),D]).filter(([D])=>!!D)),g=new Set(b.map(D=>ui(D)).filter(Boolean)),v=[...b.map((D,K)=>{let te=ui(D),re=te&&$.get(te)||null;return{id:$_("registry",D,K),installed:!!(re||D.installed),entry:D,extension:re}}),...p.filter(D=>{let K=ui(D);return!K||!g.has(K)}).map((D,K)=>({id:$_("installed",D,K),installed:!0,entry:null,extension:D}))].sort(AD),x=D=>qr(D.kind),w=p.filter(x),S=p.filter(D=>D.kind==="mcp_server"),k=p.filter(D=>!x(D)&&D.kind!=="mcp_server"),N=b.filter(D=>x(D)&&!D.installed),E=b.filter(D=>D.kind==="mcp_server"&&!D.installed),O=b.filter(D=>D.kind!=="mcp_server"&&!x(D)&&!D.installed),L=a.isLoading||n.isLoading,U=c.isPending||d.isPending||f.isPending;return{status:m,extensions:p,channels:w,mcpServers:S,tools:k,channelRegistry:N,mcpRegistry:E,toolRegistry:O,registry:b,catalogEntries:v,connectableChannels:y,isLoading:L,isBusy:U,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:f.mutate,invalidate:s}}function S_(e){let t=B({queryKey:["extension-setup",e?.id||e],queryFn:()=>v_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function N_(e,t){let a=J(),n=e?.id||e;return H({mutationFn:({secrets:r,fields:s})=>g_(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function __(e){let t=J(),a=e?.id||e,n=h.default.useRef(null),r=h.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=h.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=h.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(m=>m.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(m=>m.package_ref?.id===a),f=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return f==="active"||f==="ready"},[a,t]),o=h.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>TD)&&(r(),s())},ED)},[r,s,i]);return h.default.useEffect(()=>r,[r]),H({mutationFn:({secret:u,popup:c})=>y_(e,u).then(d=>({res:d,popup:c})),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url&&c&&!c.closed?c.location.href=u.authorization_url:u.authorization_url?d=window.open(u.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function k_(e,t={}){let a=B({queryKey:["pairing",e],queryFn:()=>b_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=J(),r=H({mutationFn:({code:s})=>x_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function R_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var DD={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function C_({channel:e,redeemFn:t,i18nKeys:a=DD,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=R(),o=typeof t=="function",u=k_(e,{enabled:!o}),c=J(),[d,f]=h.default.useState(""),m=MD(i,a,r),p=H({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{f("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),b=h.default.useCallback(S=>u.approve({code:S}),[u.approve]),y=h.default.useCallback(()=>{let S=d.trim();S&&(o?p.mutate({code:S}):(u.approve({code:S}),f("")))},[o,d,u.approve,p]),$=o?[]:u.requests,g=o?!1:u.isLoading,v=o?p.isPending:u.isApproving,x=o?p.isSuccess?p.data:null:u.result,w=o?p.isError?p.error:null:u.error;return g?l`
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
        ${R_(w,m.error)}
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
  `}function MD(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function ad(e){return e.package_ref?.id||""}function E_(e){return ad(e)==="slack"}function A_(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function D_(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function OD(e){let t=e||[],a=[t.find(A_),t.find(D_)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function T_({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>A_(r)?l`<${XN} action=${r.action} />`:D_(r)?l`<${Nc} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function M_({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=R(),d=t||[],f=e.enabled_channels||[],m=OD(a),p=d.some(E_),b=m.length>0&&!p;return l`
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
            statusLabel="setup"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${T_}
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
                <div key=${ad(y)} className="flex flex-col gap-3">
                  <${li}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${E_(y)&&l`<${T_}
                    slackConnectActions=${m}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&l` <${C_} channel=${ad(y)} /> `}
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
                <${zr}
                  key=${ad(y)}
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
            <${F}
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
  `}function O_({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=R(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=S_(e?.packageRef),[f,m]=h.default.useState({}),[p,b]=h.default.useState({}),y=__(e?.packageRef),$=N_(e?.packageRef,N=>{N.success!==!1&&(n&&n(N),a())}),g=h.default.useCallback(()=>{let N={};for(let[E,O]of Object.entries(f)){let L=(O||"").trim();L&&(N[E]=L)}$.mutate({secrets:N,fields:p})},[f,p,$]),v=h.default.useCallback(N=>{let E=window.open("about:blank","_blank","width=600,height=600");E&&(E.opener=null),y.mutate({secret:N,popup:E})},[y]),w=i.filter(N=>(N.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=_h(e),k=n_({extension:e,secrets:i,fields:o});return c?l`
      <${nd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(N=>l`<div
                key=${N}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${nd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${nd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${nd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
          <${M} name="bolt" className="h-3.5 w-3.5" />
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
                      <${T}
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
                onChange=${E=>m(O=>({...O,[N.name]:E.target.value}))}
                onKeyDown=${E=>E.key==="Enter"&&g()}
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
                onChange=${E=>b(O=>({...O,[N.name]:E.target.value}))}
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
        ${k&&l`
        <${T}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${w&&l`
        <${T}
          variant=${k?"secondary":"primary"}
          onClick=${g}
          disabled=${$.isPending}
        >
          ${$.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function nd({onClose:e,title:t,children:a}){return h.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),l`
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
  `}function L_(e){return e.package_ref?.id||""}function P_({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=R();return e.length===0&&t.length===0?l`
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
                  key=${L_(u)}
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
                <${zr}
                  key=${L_(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function LD(e){return e?.package_ref?.id||""}function PD(e){return e.entry||e.extension||{}}function U_({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=R(),[o,u]=h.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let $=PD(y);return($.display_name||LD($)).toLowerCase().includes(c)||($.description||"").toLowerCase().includes(c)||($.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,f=d.filter(y=>y.installed&&y.extension),m=d.filter(y=>y.installed&&!y.extension&&y.entry),p=f.length+m.length,b=d.filter(y=>!y.installed&&y.entry);return e.length===0?l`
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
                      <${zr}
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
                      <${zr}
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
  `}function Rh(){let{tab:e="registry"}=st(),[t,a]=h.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:f,actionResult:m,clearResult:p,install:b,activate:y,remove:$,invalidate:g}=w_(),v=h.default.useCallback(N=>a(N),[]),x=h.default.useCallback(()=>a(null),[]),w=h.default.useCallback(()=>g(),[g]),S=h.default.useCallback(N=>{N&&(y(N),a(null))},[y]);if(d)return l`
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
    `;if(e==="installed")return l`<${it} to="/extensions/registry" replace />`;let k={channels:l`<${M_}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${f}
    />`,mcp:l`<${P_}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      onInstall=${b}
      isBusy=${f}
    />`,registry:l`<${U_}
      catalogEntries=${u}
      onInstall=${b}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${$}
      isBusy=${f}
    />`};return k[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${LN} result=${m} onDismiss=${p} />
          ${k[e]}
        </div>
      </div>

      ${t&&l`
        <${O_}
          extension=${t}
          onActivate=${S}
          onClose=${x}
          onSaved=${w}
        />
      `}
    </div>
  `:l`<${it} to="/extensions/registry" replace />`}var j_=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],F_=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.auto_approve_tools",labelKey:"settings.field.autoApproveTools",descKey:"settings.field.autoApproveToolsDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],q_=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],Ch=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","agent.auto_approve_tools","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function z_(e){return String(e||"").trim().toLowerCase()}function B_(e){if(e==null)return"";if(Array.isArray(e))return e.map(B_).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=z_(e);return a?t.map(B_).join(" ").toLowerCase().includes(a):!0}function di(e,t,a,n){let r=z_(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>tt(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function UD({visible:e}){let t=R();return e?l`
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      ${t("tools.saved")}
    </span>
  `:null}function jD({checked:e,onChange:t,label:a}){return l`
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
  `}function FD({field:e,value:t,onSave:a,isSaved:n}){let r=R(),[s,i]=h.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";h.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=h.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let f=parseInt(d,10);isNaN(f)||a(e.key,f)}else if(e.type==="float"){let f=parseFloat(d);isNaN(f)||a(e.key,f)}else a(e.key,d)},[e.key,e.type,a]);return l`
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">${o}</div>
        ${u&&l`<div className="mt-1 text-xs leading-5 text-iron-300">${u}</div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${e.type==="boolean"?l`
              <${jD}
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
        <${UD} visible=${n} />
      </div>
    </div>
  `}function mi({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=R(),o=t?i(t):e||"";return l`
    <${ee} className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${o}</h3>
      <div>
        ${a.map(u=>l`
              <${FD}
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
    <${ee} padding="lg">
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
  `}function I_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return l`<${qD} />`;let i=di(F_,e,r,s);return i.length===0?l`<${wt} query=${r} />`:l`
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
  `}function qD(){return l`
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
  `}function K_(){let e=B({queryKey:["gateway-status-settings"],queryFn:Hs,staleTime:1e4}),t=B({queryKey:["extensions"],queryFn:C$}),a=B({queryKey:["extension-registry"],queryFn:E$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(f=>f.kind==="wasm_channel"||f.kind==="channel"),o=s.filter(f=>(f.kind==="wasm_channel"||f.kind==="channel")&&!f.installed),u=r.filter(f=>f.kind==="mcp_server"),c=s.filter(f=>f.kind==="mcp_server"&&!f.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function zD({name:e,description:t,enabled:a,detail:n}){let r=R();return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${e}</span>
          <${F}
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
  `}function H_({channel:e,registryEntry:t}){let a=R(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${n}</span>
          ${s?l`<${F}
                tone=${o[i]||"muted"}
                label=${u[i]||i}
                size="sm"
              />`:l`<${F}
                tone="muted"
                label=${a("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${r}</div>
      </div>
    </div>
  `}function BD(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function ID({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=BD(e,i).filter(b=>tt(s,[i("channels.builtIn"),b.id,b.name,b.description,b.detail])),u=new Set(t.map(b=>b.name)),c=t.filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description,b.onboarding_state])),d=a.filter(b=>!u.has(b.name)).filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description])),f=new Set(n.map(b=>b.name)),m=n.filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description,b.active?i("channels.active"):i("channels.inactive")])),p=r.filter(b=>!f.has(b.name)).filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:p}}function Q_({searchQuery:e=""}){let t=R(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=K_();if(o)return l`
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
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:m}=ID({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&f.length===0&&m.length===0?l`<${wt} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${u.map(p=>l`
            <${zD}
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
              <${H_}
                key=${p.name}
                channel=${p}
                registryEntry=${r.find(b=>b.name===p.name)}
              />
            `)}
          ${d.map(p=>l`
              <${H_} key=${p.name} registryEntry=${p} />
            `)}
        <//>
      `}
      ${(f.length>0||m.length>0)&&l`
        <${ee} padding="md">
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
                      <${F}
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
                      <${F}
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
  `}function V_({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:f}){let m=R(),p=e.id===t,b=Lr(e,n),y=Vs(e,n),$=B$(e,n,t,a),g=mc(e,n),v=I$(e),x=m(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[w,S]=h.default.useState(p),k=h.default.useCallback(()=>S(St=>!St),[]);h.default.useEffect(()=>{S(p)},[p]);let N=b?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${Io(e.adapter)} · ${$||e.default_model||m("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${x}
      </span>`,E=e.id==="nearai"||e.id==="openai_codex",O=e.api_key_set===!0||e.has_api_key===!0,L=e.builtin?e.id==="nearai"&&v&&!O?m("llm.addApiKey"):m("llm.configure"):m("common.edit"),U=v&&e.builtin?l`
          <${T}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${L}
          <//>
        `:null,D=!p&&e.id==="nearai"?l`
          ${U}
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
        `:null,te=!p&&b&&(!E||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${T}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${m("llm.use")}
        <//>
      `:null,re=b?null:l`
        <${T}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${m(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,Ae=p?null:te||(E?D:re),Fe=!E&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
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
          aria-label=${m(w?"llm.collapseDetails":"llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${k}
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
            ${p&&l`<${F} tone="positive" label=${m("llm.active")} size="sm" />`}
            ${e.builtin&&!p&&l`<${F} tone="muted" label=${m("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${N}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${Ae}
          <button
            type="button"
            onClick=${k}
            data-testid="llm-provider-chevron"
            aria-label=${m(w?"llm.collapseDetails":"llm.expandDetails")}
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
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.adapter")}</div>
              <div className="mt-1 truncate">${Io(e.adapter)}</div>
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
            ${Fe&&l`
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
  `}var KD=[{key:"active",labelKey:"llm.groupActive",dotClass:"bg-[var(--v2-positive-text)]"},{key:"ready",labelKey:"llm.groupReady",dotClass:"bg-[var(--v2-accent)]"},{key:"setup",labelKey:"llm.groupSetup",dotClass:"bg-[var(--v2-warning-text)]"}];function HD({label:e,count:t,dotClass:a}){return l`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full "+a} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${e}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${t}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `}function G_({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=R(),r=Uc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=jc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${wt} query=${a} />`;let u=K$(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${ee} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${T} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
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

      <${Pc} login=${i} />

      ${s.isLoading?l`<div className="text-sm text-[var(--v2-text-muted)]">${n("common.loading")}</div>`:s.error?l`<div className="text-sm text-red-200">${n("error.loadFailed",{what:n("llm.providers"),message:s.error.message})}</div>`:l`
            <div className="space-y-1">
              ${KD.flatMap(c=>{let d=u[c.key];return d.length?[l`
                    <section
                      key=${c.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${c.key}
                      className="mb-3"
                    >
                      <${HD}
                        label=${n(c.labelKey)}
                        count=${d.length}
                        dotClass=${c.dotClass}
                      />
                      <div className="space-y-2">
                      ${d.map(f=>l`
                          <${V_}
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

      <${Lc}
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
  `}function Y_({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=R(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=Gs({settings:e,gatewayStatus:t});if(r)return l`<${QD} />`;let f=d?o:"",m=c.find(g=>g.id===o),p=d&&(u||m?.default_model||e.selected_model)||"",b=di(j_,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),f,i("inference.model"),p]),$=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!$&&b.length===0?l`<${wt} query=${s} />`:l`
    <div className="space-y-5">
      ${y&&l`
      <${ee} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${f||i("inference.none")}</span>
              ${d?l`<${F} tone="positive" label=${i("inference.active")} size="sm" />`:l`<${F} tone="muted" label=${i("llm.notConfigured")} size="sm" />`}
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
        <${G_}
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
  `}function QD(){return l`
    <div className="space-y-5">
      <${ee} padding="md">
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
            <${ee} key=${e} padding="md">
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
  `}function J_({searchQuery:e=""}){let t=R(),{lang:a,setLang:n}=rl(),r=sl.find(i=>i.code===a)||sl[0],s=sl.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?l`<${wt} query=${e} />`:l`
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
  `}function X_({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return l`
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
    `;let i=di(q_,e,r,s);return i.length===0?l`<${wt} query=${r} />`:l`
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
  `}function Z_(){let e=R(),[t,a]=h.default.useState(!1),n=h.default.useCallback(()=>a(!0),[]),r=h.default.useCallback(()=>a(!1),[]),s=h.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function W_({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=R(),r=Z_({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
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

        <${T}
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
  `:null}function ek(){let e=J(),t=B({queryKey:["skills"],queryFn:T$}),a=H({mutationFn:D$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=H({mutationFn:O$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=H({mutationFn:({name:c,content:d})=>M$(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=H({mutationFn:({name:c,enabled:d})=>L$(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=H({mutationFn:c=>P$(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],u=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:u,fetchSkillContent:A$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function tk({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let u=R(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",f=e.source_kind||"installed",m=!!e.can_edit,p=!!e.can_delete,b=e.auto_activate!==!1,[y,$]=h.default.useState(!1),[g,v]=h.default.useState(""),[x,w]=h.default.useState(""),[S,k]=h.default.useState(!1);h.default.useEffect(()=>{y||(v(""),w(""))},[y]);let N=h.default.useCallback(async()=>{k(!0),w("");try{let O=await t(c);v(O?.content||""),$(!0)}catch(O){w(O.message||u("skills.contentLoadFailed"))}finally{k(!1)}},[c,t,u]),E=h.default.useCallback(async()=>{(await n(c,g))?.success&&$(!1)},[g,c,n]);return l`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${c}</span>
            <${F}
              tone=${String(d).toLowerCase()==="trusted"?"positive":"muted"}
              label=${d}
              size="sm"
            />
            <${F}
              tone=${f==="system"?"positive":"muted"}
              label=${u(`skills.source.${f}`)}
              size="sm"
            />
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?l`
                <div className="mt-3">
                  <${wc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${O=>v(O.currentTarget.value)}
                  />
                </div>
              `:l`<${VD} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${m&&!y&&l`
            <${T}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${u("skills.edit")}
              onClick=${N}
            >
              <${M} name="file" className="h-4 w-4" />
              ${u(S?"skills.loading":"skills.edit")}
            <//>
          `}
          ${y&&l`
            <${T}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${i}
              onClick=${()=>{v(""),$(!1)}}
            >
              <${M} name="close" className="h-4 w-4" />
              ${u("skills.cancel")}
            <//>
            <${T}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${E}
            >
              <${M} name="check" className="h-4 w-4" />
              ${u(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${m&&!y&&l`
            <${T}
              type="button"
              variant=${b?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${u(b?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!b)}
            >
              <${M} name=${b?"check":"close"} className="h-4 w-4" />
              ${u(b?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
            <//>
          `}
          ${p&&!y&&l`
            <${T}
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
      ${x&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${x}</p>`}
    </div>
  `}function VD({skill:e}){let t=R();return l`
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
        ${e.has_requirements&&l`<${Eh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${Eh}>scripts/<//>`}
        ${e.install_source_url&&l`<${Eh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function Eh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function ak({onInstall:e,isInstalling:t}){let a=R(),[n,r]=h.default.useState(""),[s,i]=h.default.useState(""),[o,u]=h.default.useState({name:"",content:""}),[c,d]=h.default.useState(""),[f,m]=h.default.useState(""),p=h.default.useCallback((y,$)=>{u(g=>!g[y]||!$.trim()?g:{...g,[y]:""})},[]),b=h.default.useCallback(async()=>{let y=GD({name:n,content:s}),$=YD(y,a);if($.name||$.content){u($),d(""),m("");return}u({name:"",content:""}),d(""),m("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),m(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
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

      <${bn} label=${a("skills.name")} error=${o.name} required>
        <${Tt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let $=y.currentTarget.value;r($),p("name",$)}}
        />
      <//>

      <${bn}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${wc}
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
          <${M} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function GD({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function YD(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function nk({searchQuery:e=""}){let t=R(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:u,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:f,isRemoving:m,isUpdating:p,isSettingAutoActivate:b,isSettingAutoActivateLearned:y}=ek(),[$,g]=h.default.useState(""),[v,x]=h.default.useState(""),w=h.default.useCallback(async O=>{if(window.confirm(t("skills.confirmDelete",{name:O}))){g(""),x("");try{let L=await o(O);if(!L?.success){g(L?.message||t("skills.removeFailed"));return}x(L.message||t("skills.removed",{name:O}))}catch(L){g(L.message||t("skills.removeFailed"))}}},[o,t]),S=h.default.useCallback(async(O,L)=>{if(!L.trim())return g(t("skills.contentRequired")),x(""),{success:!1,message:t("skills.contentRequired")};g(""),x("");try{let U=await u({name:O,content:L});return U?.success?(x(U.message||t("skills.updated",{name:O})),U):(g(U?.message||t("skills.updateFailed")),U)}catch(U){let D=U.message||t("skills.updateFailed");return g(D),{success:!1,message:D}}},[t,u]),k=h.default.useCallback(async(O,L)=>{g(""),x("");try{let U=await c({name:O,enabled:L});if(!U?.success){g(U?.message||t("skills.updateFailed"));return}x(U.message)}catch(U){g(U.message||t("skills.updateFailed"))}},[c,t]),N=h.default.useCallback(async O=>{g(""),x("");try{let L=await d(O);if(!L?.success){g(L?.message||t("skills.updateFailed"));return}x(L.message)}catch(L){g(L.message||t("skills.updateFailed"))}},[d,t]),E;if(n.isLoading)E=l`
      <${ee} padding="md">
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
    `;else if(n.error)E=l`
      <${ee} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let O=a.filter(U=>tt(e,[U.name,U.id,U.description,U.keywords,U.trust_level,U.source_kind,U.version])),L=ZD(O);a.length===0?E=l`
        <${ee} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:O.length===0?E=l`<${wt} query=${e} />`:E=l`
        <div id="skills-list">
          ${L.map(U=>l`
              <${XD}
                key=${U.id}
                title=${t(U.labelKey)}
                skills=${U.skills}
                onEdit=${s}
                onRemove=${w}
                onUpdate=${S}
                onSetAutoActivate=${k}
                isRemoving=${m}
                isUpdating=${p}
                isSettingAutoActivate=${b}
              />
            `)}
        </div>
      `}return l`
    <div className="space-y-4">
      <${JD}
        enabled=${r}
        isSaving=${y}
        onToggle=${N}
      />
      <${ak} onInstall=${i} isInstalling=${f} />
      <${WD} error=${$} result=${v} />
      ${E}
    </div>
  `}function JD({enabled:e,isSaving:t,onToggle:a}){let n=R();return l`
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
  `}function XD({title:e,skills:t,onEdit:a,onRemove:n,onUpdate:r,onSetAutoActivate:s,isRemoving:i,isUpdating:o,isSettingAutoActivate:u}){return t.length===0?null:l`
    <${ee} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>l`
          <${tk}
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
  `}function ZD(e){let t=[{id:"user",labelKey:"skills.group.user",skills:[]},{id:"system",labelKey:"skills.group.system",skills:[]},{id:"workspace",labelKey:"skills.group.workspace",skills:[]}],a=t[0];for(let n of e){let r=n.source_kind||"";(r==="system"?t[1]:r==="workspace"?t[2]:a).skills.push(n)}return t.filter(n=>n.skills.length>0)}function WD({error:e,result:t}){return!e&&!t?null:l`
    <div
      className=${e?"rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200":"rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${e||t}
    </div>
  `}function rd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function rk(){let e=J(),t=B({queryKey:["settings-tools"],queryFn:k$}),a=t.data?.tools||[],[n,r]=h.default.useState({}),s=H({mutationFn:async({name:o,state:u})=>rd(await R$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>d&&{...d,tools:d.tools.map(f=>f.name===u?{...f,state:c}:f)}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=h.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}function eM({tool:e,onPermissionChange:t,isSaved:a}){let n=R(),r=[{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s=e.locked,i=r.find(u=>u.value===e.state)||r[1],o=e.state===e.default_state;return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${s&&l`<${M}
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
        ${s?l`<${F} tone=${i.tone} label=${i.label} size="sm" />`:l`
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
  `}function sk({searchQuery:e=""}){let t=R(),{tools:a,query:n,setPermission:r,savedTools:s}=rk();if(n.isLoading)return l`
      <${ee} padding="md">
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
      <${ee} padding="md">
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

      <${ee} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("tools.permissions")}
        </h3>
        ${i.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("tools.noMatch")}
            </p>`:i.map(o=>l`
                  <${eM}
                    key=${o.name}
                    tool=${o}
                    onPermissionChange=${r}
                    isSaved=${s[o.name]}
                  />
                `)}
      <//>
    </div>
  `}function ik(e){return(Number(e)||0).toFixed(2)}function tM(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function ok(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Br({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function lk({searchQuery:e=""}){let t=R(),{credits:a,query:n,authorize:r}=hc();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${wt} query=${e} />`;let s;if(n.isLoading)s=l`
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
          value=${ik(a.pending_credit)}
        />
        <${Br}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${ik(a.final_credit)}
        />
        <${Br}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${tM(a.delayed_credit_delta)}
        />
        <${Br}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Br}
          label=${t("traceCommons.lastSubmission")}
          value=${ok(a.last_submission_at,t)}
        />
        <${Br}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${ok(a.last_credit_sync_at,t)}
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
  `}function uk(){let e=J(),t=B({queryKey:["admin-users"],queryFn:F$,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=H({mutationFn:q$,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=H({mutationFn:({id:i,payload:o})=>z$(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function aM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=p=>{p.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),f(!1)}})};return d?l`
    <${ee} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${m} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${bn} label=${n("users.displayName")} htmlFor="user-name">
            <${Tt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${p=>s(p.target.value)}
              required
            />
          <//>
          <${bn} label=${n("users.email")} htmlFor="user-email">
            <${Tt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${p=>o(p.target.value)}
            />
          <//>
        </div>
        <${bn} label=${n("users.role")} htmlFor="user-role">
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
        <${M} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function nM({user:e}){let t=R(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${e.display_name||e.id}</span
          >
          <${F}
            tone=${n}
            label=${e.role==="admin"?t("users.admin"):t("users.member")}
            size="sm"
          />
          <${F} tone=${a} label=${e.status||"active"} size="sm" />
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
  `}function ck({searchQuery:e=""}){let t=R(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=uk();if(n.isLoading)return l`
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
      <${ee} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let u=a.filter(c=>tt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${aM}
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
            </p>`:u.map(c=>l`<${nM} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function dk(){let e=J(),t=B({queryKey:["settings-export"],queryFn:v$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=h.default.useState({}),[s,i]=h.default.useState(!1),o=H({mutationFn:async({key:f,value:m})=>rd(await g$(f,m),"Save failed"),onSuccess:(f,{key:m,value:p})=>{e.setQueryData(["settings-export"],b=>{if(!b)return b;let y={...b,settings:{...b.settings}};return p==null?delete y.settings[m]:y.settings[m]=p,y}),r(b=>({...b,[m]:!0})),setTimeout(()=>r(b=>({...b,[m]:!1})),2e3),Ch.has(m)&&i(!0)}}),u=h.default.useCallback((f,m)=>o.mutate({key:f,value:m}),[o]),c=H({mutationFn:y$,onSuccess:(f,m)=>{e.invalidateQueries({queryKey:["settings-export"]}),Object.keys(m?.settings||{}).some(b=>Ch.has(b))&&i(!0)}}),d=h.default.useCallback(f=>c.mutateAsync(f),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Th(){let e=R(),{tab:t}=st(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=Ba(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:f,saveError:m}=dk(),[p,b]=h.default.useState("");h.default.useEffect(()=>{b("")},[i]);let y=u.isLoading,$={inference:l`<${Y_}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,agent:l`<${I_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,channels:l`<${Q_} searchQuery=${p} />`,networking:l`<${X_}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${p}
    />`,tools:l`<${sk} searchQuery=${p} />`,skills:l`<${nk} searchQuery=${p} />`,traces:l`<${lk} searchQuery=${p} />`,users:l`<${ck} searchQuery=${p} />`,language:l`<${J_} searchQuery=${p} />`},g=k=>k==="users"||k==="inference",v=k=>Object.prototype.hasOwnProperty.call($,k),x=Object.keys($).filter(k=>r||!g(k)),S=v(s)&&x.includes(s)?s:x[0]||"language";return!v(i)||!r&&g(i)?l`<${it} to=${`/settings/${S}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${f&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${W_}
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
  `}var Ah=Object.freeze({todo:!0});function mk(){return Promise.resolve({users:[],total:0,...Ah})}function fk(e){return Promise.resolve(null)}function pk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function hk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function vk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function gk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function yk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function bk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function xk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Ah})}function $k(e="day",t){return Promise.resolve({entries:[],...Ah})}function wk(){return B({queryKey:["admin","usage-summary"],queryFn:xk,refetchInterval:3e4})}function sd(e="day",t){return B({queryKey:["admin","usage",e,t],queryFn:()=>$k(e,t),refetchInterval:3e4})}function fi(){let e=J(),t=B({queryKey:["admin","users"],queryFn:mk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=H({mutationFn:pk,onSuccess:s}),o=H({mutationFn:({id:m,payload:p})=>hk(m,p),onSuccess:s}),u=H({mutationFn:m=>vk(m),onSuccess:s}),c=H({mutationFn:m=>gk(m),onSuccess:s}),d=H({mutationFn:m=>yk(m),onSuccess:s}),f=H({mutationFn:({userId:m,name:p})=>bk(m,p)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(m,p)=>o.mutateAsync({id:m,payload:p}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(m,p)=>f.mutateAsync({userId:m,name:p}),newToken:f.data,clearToken:()=>f.reset()}}function Sk(e){return B({queryKey:["admin","user",e],queryFn:()=>fk(e),enabled:!!e,refetchInterval:1e4})}function Ga(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Ra(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Nk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function sr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function pi(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function hi(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function vi(e){return e==="admin"?"signal":"muted"}function _k(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function kk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Rk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Ck(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Ek(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function rM({users:e,onSelectUser:t}){let a=R(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
                <td className="py-3 pr-4"><${F} tone=${vi(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${F} tone=${hi(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${sr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function Tk({onSelectUser:e,onNavigateTab:t}){let a=R(),n=wk(),{users:r,query:s}=fi(),i=n.data||{},o=_k(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
      <div className="space-y-5">
        <${z} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:l`
    <div className="space-y-5">
      <${z} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&l`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Nk(i.uptime_seconds)})}</span>
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

      <${z} className="p-5 sm:p-6">
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

      <${z} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.recentUsers")}</h3>
          <button
            onClick=${()=>t("users")}
            className="text-xs text-signal hover:underline"
          >
            ${a("admin.dashboard.viewAll")}
          </button>
        </div>
        <${rM} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var sM=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function iM({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function Ak({onSelectUser:e}){let t=R(),[a,n]=h.default.useState("day"),r=sd(a),s=r.data?.usage||[],i=Rk(s),o=Ck(s),u=Ek(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
      <${z} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1,2,3,4].map(d=>l`<div key=${d} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `:l`
    <div className="space-y-5">
      <${z} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${sM.map(d=>l`
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
                <${et} label=${t("admin.usage.inputTokens")} value=${Ga(u.input_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.outputTokens")} value=${Ga(u.output_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.totalCost")} value=${Ra(u.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${i.length>0&&l`
        <${z} className="p-5 sm:p-6">
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ga(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ga(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Ra(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${iM} value=${d.cost} max=${c} />
                      </td>
                    </tr>
                  `)}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${o.length>0&&l`
        <${z} className="p-5 sm:p-6">
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ga(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ga(d.output_tokens)}</td>
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
  `}function Dk({userId:e,onBack:t}){let a=R(),n=Sk(e),r=sd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:f}=fi(),[m,p]=h.default.useState(null),[b,y]=h.default.useState(!1),$=n.data,g=r.data?.usage||[];if(h.default.useEffect(()=>{$&&m===null&&p($.role)},[$]),n.isLoading)return l`
      <div className="space-y-5">
        <${z} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        <//>
      </div>
    `;if(n.error)return l`
      <${z} className="p-5 sm:p-6">
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

      <${z} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">${$.display_name||$.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${F} tone=${vi($.role)} label=${$.role||"member"} />
              <${F} tone=${hi($.status)} label=${$.status||"active"} />
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
              <${M} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${z} className="p-5 sm:p-6">
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

        <${z} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${ir} label=${a("admin.user.jobs")}>${$.job_count??0}<//>
          <${ir} label=${a("admin.user.totalCost")}>${Ra($.total_cost)}<//>
          <${ir} label=${a("admin.user.lastActive")}>${sr($.last_active_at)}<//>
        <//>
      </div>

      <${z} className="p-5 sm:p-6">
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

      <${z} className="p-5 sm:p-6">
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
                    ${g.map((S,k)=>l`
                        <tr key=${k} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ga(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Ga(S.output_tokens)}</td>
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
  `}function oM(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function lM({token:e,onDismiss:t}){let a=R(),[n,r]=h.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
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
          <${M} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function uM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=h.default.useState(""),[i,o]=h.default.useState(""),[u,c]=h.default.useState("member"),[d,f]=h.default.useState(!1),m=async p=>{p.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),f(!1))};return d?l`
    <${z} className="p-5 sm:p-6">
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
        <${M} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function cM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=R();return l`
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
  `}function dM({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=R();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${F} tone=${vi(e.role)} label=${e.role||"member"} />
          <${F} tone=${hi(e.status)} label=${e.status||"active"} />
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
  `}function Mk({selectedUserId:e,onSelectUser:t}){let a=R(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:f,activateUser:m,createToken:p,newToken:b,clearToken:y}=fi(),[$,g]=h.default.useState(""),[v,x]=h.default.useState("all"),[w,S]=h.default.useState(null),k=kk(n,{search:$,filter:v}),N=oM(a),E=L=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{f(L),S(null)}})},O=async(L,U)=>{let D=window.prompt(a("admin.users.tokenNamePrompt",{name:U||a("admin.users.userFallback")}));D&&await p(L,D)};return r.isLoading?l`
      <${z} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(L=>l`
          <div key=${L} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?l`
      <${z} className="p-6 sm:p-8">
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
      ${b&&l`
        <${lM}
          token=${b.token||b.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${uM} onCreate=${i} isCreating=${o} error=${u} />

      <${z} className="p-5 sm:p-6">
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
              ${N.map(L=>l`
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

        ${k.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:k.map(L=>l`
                <${dM}
                  key=${L.id}
                  user=${L}
                  onSelect=${t}
                  onSuspend=${E}
                  onActivate=${m}
                  onChangeRole=${(U,D)=>c(U,{role:D})}
                  onCreateToken=${O}
                />
              `)}
      <//>

      ${w&&l`
        <${cM}
          title=${w.title}
          message=${w.message}
          confirmLabel=${w.confirmLabel}
          onConfirm=${w.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function Ok(){let{tab:e="dashboard"}=st(),t=de(),[a,n]=h.default.useState(null),r=h.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=h.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${Tk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${Dk} userId=${a} onBack=${s} />`:l`<${Mk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${Ak} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${it} to="/admin/dashboard" replace />`}var mM=2e3,fM=500,pM=2e3,hM=new Set([403,404]),vM=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function gM(e=globalThis.location){let t=new URLSearchParams(e?.search||"");return vM.reduce((a,[n,r,s])=>{let i=t.get(r)?.trim();return i?(a[n]=i,a.active.push({key:n,param:r,labelKey:s,value:i})):a[n]=null,a},{active:[]})}function Lk(){let e=Pe(),t=h.default.useMemo(()=>gM(e),[e.search]),[a,n]=h.default.useState([]),[r,s]=h.default.useState("all"),[i,o]=h.default.useState(""),[u,c]=h.default.useState(!1),[d,f]=h.default.useState(!0),[m,p]=h.default.useState(!0),[b,y]=h.default.useState(null),[$,g]=h.default.useState(!1),v=h.default.useRef(new Set),x=h.default.useRef(0),w=h.default.useCallback(async()=>{if($)return;let N=++x.current;p(!0);try{let E=await Ex({limit:fM,level:r==="all"?null:r,target:i.trim()||null,threadId:t.threadId,runId:t.runId,turnId:t.turnId,toolCallId:t.toolCallId,toolName:t.toolName,source:t.source});if(N!==x.current)return;let O=v.current,U=wN(E).entries.filter(D=>!O.has(D.id));n(U),y(null)}catch(E){if(N!==x.current)return;if(hM.has(E?.status)){n([]),y(null),g(!0);return}y(E)}finally{N===x.current&&p(!1)}},[$,r,t,i]);h.default.useEffect(()=>{w()},[w]),h.default.useEffect(()=>{if(u||$)return;let N=setInterval(w,mM);return()=>clearInterval(N)},[$,w,u]);let S=h.default.useCallback(()=>{c(N=>!N)},[]),k=h.default.useCallback(()=>{let N=[...v.current,...a.map(E=>E.id)].slice(-pM);v.current=new Set(N),n([])},[a]);return{entries:a,totalCount:a.length,paused:u,togglePause:S,clearEntries:k,levelFilter:r,setLevelFilter:s,targetFilter:i,setTargetFilter:o,autoScroll:d,setAutoScroll:f,serverLevel:null,changeServerLevel:async()=>{},scope:t,status:b?"error":m?"loading":"ready",isLoading:m,error:b}}var yM=["all","trace","debug","info","warn","error"],bM=["trace","debug","info","warn","error"],Pk={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},xM={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function $M({entry:e}){let t=R(),[a,n]=h.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=Pk[e.level]||Pk.info,i=xM[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
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
  `}function Uk({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function wM({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function jk(){let e=R(),{entries:t,totalCount:a,paused:n,togglePause:r,clearEntries:s,levelFilter:i,setLevelFilter:o,targetFilter:u,setTargetFilter:c,autoScroll:d,setAutoScroll:f,serverLevel:m,changeServerLevel:p,scope:b,isLoading:y,error:$}=Lk(),g=h.default.useRef(null),v=h.default.useRef(!0);h.default.useEffect(()=>{d&&v.current&&g.current&&(g.current.scrollTop=0)},[t,d]);let x=h.default.useCallback(k=>{v.current=k.currentTarget.scrollTop<=48},[]),w=t.length>0,S=b?.active||[];return l`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${Uk}
          value=${i}
          onChange=${o}
          options=${yM}
          labelKey=${k=>k==="all"?"logs.levelAll":`logs.level.${k}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${u}
          onInput=${k=>c(k.target.value)}
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
              onChange=${k=>f(k.target.checked)}
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
            ${S.map(k=>l`<${wM} key=${k.param} scopeKey=${k.param} label=${e(k.labelKey)} value=${k.value} />`)}
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
            <${Uk}
              value=${m}
              onChange=${p}
              options=${bM}
              labelKey=${k=>`logs.level.${k}`}
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
              `:w?t.map(k=>l`<${$M} key=${k.id} entry=${k} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function qk(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function SM({auth:e}){let t=de(),n=Pe().state?.from,r=n?`${n.pathname||Or}${n.search||""}${n.hash||""}`:Or,s=`/v2${r==="/"?"":r}`,i=h.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${qk} />`:e.isAuthenticated?l`<${it} to=${r} replace />`:l`<${n1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function NM({auth:e,children:t}){let a=Pe();return e.isChecking?l`<${qk} />`:e.isAuthenticated?t:l`<${it} to="/login" replace state=${{from:a}} />`}function _M({auth:e}){return l`
    <${NM} auth=${e}>
      <${Dw}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function Fk({auth:e}){return e.isAdmin?l`<${Ok} />`:l`<${it} to=${Or} replace />`}function zk(){let e=f$();return l`
    <${gp} basename="/v2">
      <${pp}>
        <${ve} path="/login" element=${l`<${SM} auth=${e} />`} />
        <${ve} path="/" element=${l`<${_M} auth=${e} />`}>
          <${ve} index element=${l`<${it} to=${Or} replace />`} />
          <${ve} path="overview" element=${l`<${it} to=${Or} replace />`} />
          <${ve} path="welcome" element=${l`<${_2} />`} />
          <${ve} path="chat" element=${l`<${ih} />`} />
          <${ve} path="chat/:threadId" element=${l`<${ih} />`} />
          <${ve} path="workspace" element=${l`<${lh} />`} />
          <${ve} path="workspace/*" element=${l`<${lh} />`} />
          <${ve} path="projects" element=${l`<${Zo} />`} />
          <${ve} path="projects/:projectId" element=${l`<${Zo} />`} />
          <${ve} path="projects/:projectId/missions/:missionId" element=${l`<${Zo} />`} />
          <${ve} path="projects/:projectId/threads/:threadId" element=${l`<${Zo} />`} />
          <${ve} path="missions" element=${l`<${ch} />`} />
          <${ve} path="missions/:missionId" element=${l`<${ch} />`} />
          <${ve} path="jobs" element=${l`<${fh} />`} />
          <${ve} path="jobs/:jobId" element=${l`<${fh} />`} />
          <${ve} path="routines" element=${l`<${hh} />`} />
          <${ve} path="routines/:routineId" element=${l`<${hh} />`} />
          <${ve} path="automations" element=${l`<${MN} />`} />
          <${ve} path="extensions" element=${l`<${Rh} />`} />
          <${ve} path="extensions/:tab" element=${l`<${Rh} />`} />
          <${ve} path="logs" element=${l`<${jk} />`} />
          <${ve} path="settings" element=${l`<${Th} />`} />
          <${ve} path="settings/:tab" element=${l`<${Th} />`} />
          <${ve} path="admin" element=${l`<${Fk} auth=${e} />`} />
          <${ve} path="admin/:tab" element=${l`<${Fk} auth=${e} />`} />
        <//>
        <${ve} path="*" element=${l`<${it} to=${Or} replace />`} />
      <//>
    <//>
  `}Oh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveTools":"Auto-approve tools","settings.field.autoApproveToolsDesc":"Skip approval for all tool calls","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,Bk.createRoot)(document.getElementById("v2-root")).render(l`
  <${Lh}>
    <${xd} client=${Ct}>
      <${zk} />
    <//>
  <//>
`);
