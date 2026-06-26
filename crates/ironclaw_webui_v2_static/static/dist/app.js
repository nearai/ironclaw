import{a as En,b as ze,c as He,d as p,e as l,f as Qh,g as Vh,h as pl,i as R,j as hl}from"./chunks/chunk-IGTNS7XG.js";var mv=En(Nl=>{"use strict";var xR=Symbol.for("react.transitional.element"),$R=Symbol.for("react.fragment");function dv(e,t,a){var n=null;if(a!==void 0&&(n=""+a),t.key!==void 0&&(n=""+t.key),"key"in t){a={};for(var r in t)r!=="key"&&(a[r]=t[r])}else a=t;return t=a.ref,{$$typeof:xR,type:e,key:n,ref:t!==void 0?t:null,props:a}}Nl.Fragment=$R;Nl.jsx=dv;Nl.jsxs=dv});var Cd=En((XL,fv)=>{"use strict";fv.exports=mv()});var Rv=En(Me=>{"use strict";function Ld(e,t){var a=e.length;e.push(t);e:for(;0<a;){var n=a-1>>>1,r=e[n];if(0<Ml(r,t))e[n]=t,e[a]=r,a=n;else break e}}function Pa(e){return e.length===0?null:e[0]}function Ll(e){if(e.length===0)return null;var t=e[0],a=e.pop();if(a!==t){e[0]=a;e:for(var n=0,r=e.length,s=r>>>1;n<s;){var i=2*(n+1)-1,o=e[i],u=i+1,c=e[u];if(0>Ml(o,a))u<r&&0>Ml(c,o)?(e[n]=c,e[u]=a,n=u):(e[n]=o,e[i]=a,n=i);else if(u<r&&0>Ml(c,a))e[n]=c,e[u]=a,n=u;else break e}}return t}function Ml(e,t){var a=e.sortIndex-t.sortIndex;return a!==0?a:e.id-t.id}Me.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(yv=performance,Me.unstable_now=function(){return yv.now()}):(Dd=Date,bv=Dd.now(),Me.unstable_now=function(){return Dd.now()-bv});var yv,Dd,bv,an=[],Dn=[],_R=1,ma=null,St=3,Pd=!1,Mi=!1,Oi=!1,Ud=!1,wv=typeof setTimeout=="function"?setTimeout:null,Sv=typeof clearTimeout=="function"?clearTimeout:null,xv=typeof setImmediate<"u"?setImmediate:null;function Ol(e){for(var t=Pa(Dn);t!==null;){if(t.callback===null)Ll(Dn);else if(t.startTime<=e)Ll(Dn),t.sortIndex=t.expirationTime,Ld(an,t);else break;t=Pa(Dn)}}function jd(e){if(Oi=!1,Ol(e),!Mi)if(Pa(an)!==null)Mi=!0,ts||(ts=!0,es());else{var t=Pa(Dn);t!==null&&Fd(jd,t.startTime-e)}}var ts=!1,Li=-1,Nv=5,_v=-1;function kv(){return Ud?!0:!(Me.unstable_now()-_v<Nv)}function Md(){if(Ud=!1,ts){var e=Me.unstable_now();_v=e;var t=!0;try{e:{Mi=!1,Oi&&(Oi=!1,Sv(Li),Li=-1),Pd=!0;var a=St;try{t:{for(Ol(e),ma=Pa(an);ma!==null&&!(ma.expirationTime>e&&kv());){var n=ma.callback;if(typeof n=="function"){ma.callback=null,St=ma.priorityLevel;var r=n(ma.expirationTime<=e);if(e=Me.unstable_now(),typeof r=="function"){ma.callback=r,Ol(e),t=!0;break t}ma===Pa(an)&&Ll(an),Ol(e)}else Ll(an);ma=Pa(an)}if(ma!==null)t=!0;else{var s=Pa(Dn);s!==null&&Fd(jd,s.startTime-e),t=!1}}break e}finally{ma=null,St=a,Pd=!1}t=void 0}}finally{t?es():ts=!1}}}var es;typeof xv=="function"?es=function(){xv(Md)}:typeof MessageChannel<"u"?(Od=new MessageChannel,$v=Od.port2,Od.port1.onmessage=Md,es=function(){$v.postMessage(null)}):es=function(){wv(Md,0)};var Od,$v;function Fd(e,t){Li=wv(function(){e(Me.unstable_now())},t)}Me.unstable_IdlePriority=5;Me.unstable_ImmediatePriority=1;Me.unstable_LowPriority=4;Me.unstable_NormalPriority=3;Me.unstable_Profiling=null;Me.unstable_UserBlockingPriority=2;Me.unstable_cancelCallback=function(e){e.callback=null};Me.unstable_forceFrameRate=function(e){0>e||125<e?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Nv=0<e?Math.floor(1e3/e):5};Me.unstable_getCurrentPriorityLevel=function(){return St};Me.unstable_next=function(e){switch(St){case 1:case 2:case 3:var t=3;break;default:t=St}var a=St;St=t;try{return e()}finally{St=a}};Me.unstable_requestPaint=function(){Ud=!0};Me.unstable_runWithPriority=function(e,t){switch(e){case 1:case 2:case 3:case 4:case 5:break;default:e=3}var a=St;St=e;try{return t()}finally{St=a}};Me.unstable_scheduleCallback=function(e,t,a){var n=Me.unstable_now();switch(typeof a=="object"&&a!==null?(a=a.delay,a=typeof a=="number"&&0<a?n+a:n):a=n,e){case 1:var r=-1;break;case 2:r=250;break;case 5:r=1073741823;break;case 4:r=1e4;break;default:r=5e3}return r=a+r,e={id:_R++,callback:t,priorityLevel:e,startTime:a,expirationTime:r,sortIndex:-1},a>n?(e.sortIndex=a,Ld(Dn,e),Pa(an)===null&&e===Pa(Dn)&&(Oi?(Sv(Li),Li=-1):Oi=!0,Fd(jd,a-n))):(e.sortIndex=r,Ld(an,e),Mi||Pd||(Mi=!0,ts||(ts=!0,es()))),e};Me.unstable_shouldYield=kv;Me.unstable_wrapCallback=function(e){var t=St;return function(){var a=St;St=t;try{return e.apply(this,arguments)}finally{St=a}}}});var Ev=En((M6,Cv)=>{"use strict";Cv.exports=Rv()});var Av=En(Et=>{"use strict";var kR=He();function Tv(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Mn(){}var Ct={d:{f:Mn,r:function(){throw Error(Tv(522))},D:Mn,C:Mn,L:Mn,m:Mn,X:Mn,S:Mn,M:Mn},p:0,findDOMNode:null},RR=Symbol.for("react.portal");function CR(e,t,a){var n=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:RR,key:n==null?null:""+n,children:e,containerInfo:t,implementation:a}}var Pi=kR.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function Pl(e,t){if(e==="font")return"";if(typeof t=="string")return t==="use-credentials"?t:""}Et.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Ct;Et.createPortal=function(e,t){var a=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)throw Error(Tv(299));return CR(e,t,null,a)};Et.flushSync=function(e){var t=Pi.T,a=Ct.p;try{if(Pi.T=null,Ct.p=2,e)return e()}finally{Pi.T=t,Ct.p=a,Ct.d.f()}};Et.preconnect=function(e,t){typeof e=="string"&&(t?(t=t.crossOrigin,t=typeof t=="string"?t==="use-credentials"?t:"":void 0):t=null,Ct.d.C(e,t))};Et.prefetchDNS=function(e){typeof e=="string"&&Ct.d.D(e)};Et.preinit=function(e,t){if(typeof e=="string"&&t&&typeof t.as=="string"){var a=t.as,n=Pl(a,t.crossOrigin),r=typeof t.integrity=="string"?t.integrity:void 0,s=typeof t.fetchPriority=="string"?t.fetchPriority:void 0;a==="style"?Ct.d.S(e,typeof t.precedence=="string"?t.precedence:void 0,{crossOrigin:n,integrity:r,fetchPriority:s}):a==="script"&&Ct.d.X(e,{crossOrigin:n,integrity:r,fetchPriority:s,nonce:typeof t.nonce=="string"?t.nonce:void 0})}};Et.preinitModule=function(e,t){if(typeof e=="string")if(typeof t=="object"&&t!==null){if(t.as==null||t.as==="script"){var a=Pl(t.as,t.crossOrigin);Ct.d.M(e,{crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0})}}else t==null&&Ct.d.M(e)};Et.preload=function(e,t){if(typeof e=="string"&&typeof t=="object"&&t!==null&&typeof t.as=="string"){var a=t.as,n=Pl(a,t.crossOrigin);Ct.d.L(e,a,{crossOrigin:n,integrity:typeof t.integrity=="string"?t.integrity:void 0,nonce:typeof t.nonce=="string"?t.nonce:void 0,type:typeof t.type=="string"?t.type:void 0,fetchPriority:typeof t.fetchPriority=="string"?t.fetchPriority:void 0,referrerPolicy:typeof t.referrerPolicy=="string"?t.referrerPolicy:void 0,imageSrcSet:typeof t.imageSrcSet=="string"?t.imageSrcSet:void 0,imageSizes:typeof t.imageSizes=="string"?t.imageSizes:void 0,media:typeof t.media=="string"?t.media:void 0})}};Et.preloadModule=function(e,t){if(typeof e=="string")if(t){var a=Pl(t.as,t.crossOrigin);Ct.d.m(e,{as:typeof t.as=="string"&&t.as!=="script"?t.as:void 0,crossOrigin:a,integrity:typeof t.integrity=="string"?t.integrity:void 0})}else Ct.d.m(e)};Et.requestFormReset=function(e){Ct.d.r(e)};Et.unstable_batchedUpdates=function(e,t){return e(t)};Et.useFormState=function(e,t,a){return Pi.H.useFormState(e,t,a)};Et.useFormStatus=function(){return Pi.H.useHostTransitionStatus()};Et.version="19.1.0"});var Ov=En((L6,Mv)=>{"use strict";function Dv(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Dv)}catch(e){console.error(e)}}Dv(),Mv.exports=Av()});var P0=En(rc=>{"use strict";var st=Ev(),ay=He(),ER=Ov();function j(e){var t="https://react.dev/errors/"+e;if(1<arguments.length){t+="?args[]="+encodeURIComponent(arguments[1]);for(var a=2;a<arguments.length;a++)t+="&args[]="+encodeURIComponent(arguments[a])}return"Minified React error #"+e+"; visit "+t+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function ny(e){return!(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)}function No(e){var t=e,a=e;if(e.alternate)for(;t.return;)t=t.return;else{e=t;do t=e,(t.flags&4098)!==0&&(a=t.return),e=t.return;while(e)}return t.tag===3?a:null}function ry(e){if(e.tag===13){var t=e.memoizedState;if(t===null&&(e=e.alternate,e!==null&&(t=e.memoizedState)),t!==null)return t.dehydrated}return null}function Lv(e){if(No(e)!==e)throw Error(j(188))}function TR(e){var t=e.alternate;if(!t){if(t=No(e),t===null)throw Error(j(188));return t!==e?null:e}for(var a=e,n=t;;){var r=a.return;if(r===null)break;var s=r.alternate;if(s===null){if(n=r.return,n!==null){a=n;continue}break}if(r.child===s.child){for(s=r.child;s;){if(s===a)return Lv(r),e;if(s===n)return Lv(r),t;s=s.sibling}throw Error(j(188))}if(a.return!==n.return)a=r,n=s;else{for(var i=!1,o=r.child;o;){if(o===a){i=!0,a=r,n=s;break}if(o===n){i=!0,n=r,a=s;break}o=o.sibling}if(!i){for(o=s.child;o;){if(o===a){i=!0,a=s,n=r;break}if(o===n){i=!0,n=s,a=r;break}o=o.sibling}if(!i)throw Error(j(189))}}if(a.alternate!==n)throw Error(j(190))}if(a.tag!==3)throw Error(j(188));return a.stateNode.current===a?e:t}function sy(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e;for(e=e.child;e!==null;){if(t=sy(e),t!==null)return t;e=e.sibling}return null}var Ae=Object.assign,AR=Symbol.for("react.element"),Ul=Symbol.for("react.transitional.element"),Hi=Symbol.for("react.portal"),ls=Symbol.for("react.fragment"),iy=Symbol.for("react.strict_mode"),gm=Symbol.for("react.profiler"),DR=Symbol.for("react.provider"),oy=Symbol.for("react.consumer"),ln=Symbol.for("react.context"),ff=Symbol.for("react.forward_ref"),ym=Symbol.for("react.suspense"),bm=Symbol.for("react.suspense_list"),pf=Symbol.for("react.memo"),Pn=Symbol.for("react.lazy");Symbol.for("react.scope");var xm=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var MR=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var Pv=Symbol.iterator;function Ui(e){return e===null||typeof e!="object"?null:(e=Pv&&e[Pv]||e["@@iterator"],typeof e=="function"?e:null)}var OR=Symbol.for("react.client.reference");function $m(e){if(e==null)return null;if(typeof e=="function")return e.$$typeof===OR?null:e.displayName||e.name||null;if(typeof e=="string")return e;switch(e){case ls:return"Fragment";case gm:return"Profiler";case iy:return"StrictMode";case ym:return"Suspense";case bm:return"SuspenseList";case xm:return"Activity"}if(typeof e=="object")switch(e.$$typeof){case Hi:return"Portal";case ln:return(e.displayName||"Context")+".Provider";case oy:return(e._context.displayName||"Context")+".Consumer";case ff:var t=e.render;return e=e.displayName,e||(e=t.displayName||t.name||"",e=e!==""?"ForwardRef("+e+")":"ForwardRef"),e;case pf:return t=e.displayName||null,t!==null?t:$m(e.type)||"Memo";case Pn:t=e._payload,e=e._init;try{return $m(e(t))}catch{}}return null}var Qi=Array.isArray,ne=ay.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,ge=ER.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,xr={pending:!1,data:null,method:null,action:null},wm=[],us=-1;function Ia(e){return{current:e}}function mt(e){0>us||(e.current=wm[us],wm[us]=null,us--)}function Le(e,t){us++,wm[us]=e.current,e.current=t}var Ba=Ia(null),uo=Ia(null),Qn=Ia(null),fu=Ia(null);function pu(e,t){switch(Le(Qn,t),Le(uo,e),Le(Ba,null),t.nodeType){case 9:case 11:e=(e=t.documentElement)&&(e=e.namespaceURI)?qg(e):0;break;default:if(e=t.tagName,t=t.namespaceURI)t=qg(t),e=N0(t,e);else switch(e){case"svg":e=1;break;case"math":e=2;break;default:e=0}}mt(Ba),Le(Ba,e)}function Cs(){mt(Ba),mt(uo),mt(Qn)}function Sm(e){e.memoizedState!==null&&Le(fu,e);var t=Ba.current,a=N0(t,e.type);t!==a&&(Le(uo,e),Le(Ba,a))}function hu(e){uo.current===e&&(mt(Ba),mt(uo)),fu.current===e&&(mt(fu),xo._currentValue=xr)}var Nm=Object.prototype.hasOwnProperty,hf=st.unstable_scheduleCallback,Bd=st.unstable_cancelCallback,LR=st.unstable_shouldYield,PR=st.unstable_requestPaint,za=st.unstable_now,UR=st.unstable_getCurrentPriorityLevel,ly=st.unstable_ImmediatePriority,uy=st.unstable_UserBlockingPriority,vu=st.unstable_NormalPriority,jR=st.unstable_LowPriority,cy=st.unstable_IdlePriority,FR=st.log,BR=st.unstable_setDisableYieldValue,_o=null,Zt=null;function qn(e){if(typeof FR=="function"&&BR(e),Zt&&typeof Zt.setStrictMode=="function")try{Zt.setStrictMode(_o,e)}catch{}}var Wt=Math.clz32?Math.clz32:IR,zR=Math.log,qR=Math.LN2;function IR(e){return e>>>=0,e===0?32:31-(zR(e)/qR|0)|0}var jl=256,Fl=4194304;function gr(e){var t=e&42;if(t!==0)return t;switch(e&-e){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e&4194048;case 4194304:case 8388608:case 16777216:case 33554432:return e&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return e}}function Iu(e,t,a){var n=e.pendingLanes;if(n===0)return 0;var r=0,s=e.suspendedLanes,i=e.pingedLanes;e=e.warmLanes;var o=n&134217727;return o!==0?(n=o&~s,n!==0?r=gr(n):(i&=o,i!==0?r=gr(i):a||(a=o&~e,a!==0&&(r=gr(a))))):(o=n&~s,o!==0?r=gr(o):i!==0?r=gr(i):a||(a=n&~e,a!==0&&(r=gr(a)))),r===0?0:t!==0&&t!==r&&(t&s)===0&&(s=r&-r,a=t&-t,s>=a||s===32&&(a&4194048)!==0)?t:r}function ko(e,t){return(e.pendingLanes&~(e.suspendedLanes&~e.pingedLanes)&t)===0}function KR(e,t){switch(e){case 1:case 2:case 4:case 8:case 64:return t+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return t+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function dy(){var e=jl;return jl<<=1,(jl&4194048)===0&&(jl=256),e}function my(){var e=Fl;return Fl<<=1,(Fl&62914560)===0&&(Fl=4194304),e}function zd(e){for(var t=[],a=0;31>a;a++)t.push(e);return t}function Ro(e,t){e.pendingLanes|=t,t!==268435456&&(e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0)}function HR(e,t,a,n,r,s){var i=e.pendingLanes;e.pendingLanes=a,e.suspendedLanes=0,e.pingedLanes=0,e.warmLanes=0,e.expiredLanes&=a,e.entangledLanes&=a,e.errorRecoveryDisabledLanes&=a,e.shellSuspendCounter=0;var o=e.entanglements,u=e.expirationTimes,c=e.hiddenUpdates;for(a=i&~a;0<a;){var d=31-Wt(a),f=1<<d;o[d]=0,u[d]=-1;var m=c[d];if(m!==null)for(c[d]=null,d=0;d<m.length;d++){var h=m[d];h!==null&&(h.lane&=-536870913)}a&=~f}n!==0&&fy(e,n,0),s!==0&&r===0&&e.tag!==0&&(e.suspendedLanes|=s&~(i&~t))}function fy(e,t,a){e.pendingLanes|=t,e.suspendedLanes&=~t;var n=31-Wt(t);e.entangledLanes|=t,e.entanglements[n]=e.entanglements[n]|1073741824|a&4194090}function py(e,t){var a=e.entangledLanes|=t;for(e=e.entanglements;a;){var n=31-Wt(a),r=1<<n;r&t|e[n]&t&&(e[n]|=t),a&=~r}}function vf(e){switch(e){case 2:e=1;break;case 8:e=4;break;case 32:e=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:e=128;break;case 268435456:e=134217728;break;default:e=0}return e}function gf(e){return e&=-e,2<e?8<e?(e&134217727)!==0?32:268435456:8:2}function hy(){var e=ge.p;return e!==0?e:(e=window.event,e===void 0?32:O0(e.type))}function QR(e,t){var a=ge.p;try{return ge.p=e,t()}finally{ge.p=a}}var nr=Math.random().toString(36).slice(2),Nt="__reactFiber$"+nr,qt="__reactProps$"+nr,Fs="__reactContainer$"+nr,_m="__reactEvents$"+nr,VR="__reactListeners$"+nr,GR="__reactHandles$"+nr,Uv="__reactResources$"+nr,Co="__reactMarker$"+nr;function yf(e){delete e[Nt],delete e[qt],delete e[_m],delete e[VR],delete e[GR]}function cs(e){var t=e[Nt];if(t)return t;for(var a=e.parentNode;a;){if(t=a[Fs]||a[Nt]){if(a=t.alternate,t.child!==null||a!==null&&a.child!==null)for(e=Hg(e);e!==null;){if(a=e[Nt])return a;e=Hg(e)}return t}e=a,a=e.parentNode}return null}function Bs(e){if(e=e[Nt]||e[Fs]){var t=e.tag;if(t===5||t===6||t===13||t===26||t===27||t===3)return e}return null}function Vi(e){var t=e.tag;if(t===5||t===26||t===27||t===6)return e.stateNode;throw Error(j(33))}function xs(e){var t=e[Uv];return t||(t=e[Uv]={hoistableStyles:new Map,hoistableScripts:new Map}),t}function ct(e){e[Co]=!0}var vy=new Set,gy={};function Ar(e,t){Es(e,t),Es(e+"Capture",t)}function Es(e,t){for(gy[e]=t,e=0;e<t.length;e++)vy.add(t[e])}var YR=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),jv={},Fv={};function JR(e){return Nm.call(Fv,e)?!0:Nm.call(jv,e)?!1:YR.test(e)?Fv[e]=!0:(jv[e]=!0,!1)}function eu(e,t,a){if(JR(t))if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":e.removeAttribute(t);return;case"boolean":var n=t.toLowerCase().slice(0,5);if(n!=="data-"&&n!=="aria-"){e.removeAttribute(t);return}}e.setAttribute(t,""+a)}}function Bl(e,t,a){if(a===null)e.removeAttribute(t);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(t);return}e.setAttribute(t,""+a)}}function nn(e,t,a,n){if(n===null)e.removeAttribute(a);else{switch(typeof n){case"undefined":case"function":case"symbol":case"boolean":e.removeAttribute(a);return}e.setAttributeNS(t,a,""+n)}}var qd,Bv;function ss(e){if(qd===void 0)try{throw Error()}catch(a){var t=a.stack.trim().match(/\n( *(at )?)/);qd=t&&t[1]||"",Bv=-1<a.stack.indexOf(`
    at`)?" (<anonymous>)":-1<a.stack.indexOf("@")?"@unknown:0:0":""}return`
`+qd+e+Bv}var Id=!1;function Kd(e,t){if(!e||Id)return"";Id=!0;var a=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var n={DetermineComponentFrameRoot:function(){try{if(t){var f=function(){throw Error()};if(Object.defineProperty(f.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(f,[])}catch(h){var m=h}Reflect.construct(e,[],f)}else{try{f.call()}catch(h){m=h}e.call(f.prototype)}}else{try{throw Error()}catch(h){m=h}(f=e())&&typeof f.catch=="function"&&f.catch(function(){})}}catch(h){if(h&&m&&typeof h.stack=="string")return[h.stack,m.stack]}return[null,null]}};n.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var r=Object.getOwnPropertyDescriptor(n.DetermineComponentFrameRoot,"name");r&&r.configurable&&Object.defineProperty(n.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var s=n.DetermineComponentFrameRoot(),i=s[0],o=s[1];if(i&&o){var u=i.split(`
`),c=o.split(`
`);for(r=n=0;n<u.length&&!u[n].includes("DetermineComponentFrameRoot");)n++;for(;r<c.length&&!c[r].includes("DetermineComponentFrameRoot");)r++;if(n===u.length||r===c.length)for(n=u.length-1,r=c.length-1;1<=n&&0<=r&&u[n]!==c[r];)r--;for(;1<=n&&0<=r;n--,r--)if(u[n]!==c[r]){if(n!==1||r!==1)do if(n--,r--,0>r||u[n]!==c[r]){var d=`
`+u[n].replace(" at new "," at ");return e.displayName&&d.includes("<anonymous>")&&(d=d.replace("<anonymous>",e.displayName)),d}while(1<=n&&0<=r);break}}}finally{Id=!1,Error.prepareStackTrace=a}return(a=e?e.displayName||e.name:"")?ss(a):""}function XR(e){switch(e.tag){case 26:case 27:case 5:return ss(e.type);case 16:return ss("Lazy");case 13:return ss("Suspense");case 19:return ss("SuspenseList");case 0:case 15:return Kd(e.type,!1);case 11:return Kd(e.type.render,!1);case 1:return Kd(e.type,!0);case 31:return ss("Activity");default:return""}}function zv(e){try{var t="";do t+=XR(e),e=e.return;while(e);return t}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}function pa(e){switch(typeof e){case"bigint":case"boolean":case"number":case"string":case"undefined":return e;case"object":return e;default:return""}}function yy(e){var t=e.type;return(e=e.nodeName)&&e.toLowerCase()==="input"&&(t==="checkbox"||t==="radio")}function ZR(e){var t=yy(e)?"checked":"value",a=Object.getOwnPropertyDescriptor(e.constructor.prototype,t),n=""+e[t];if(!e.hasOwnProperty(t)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var r=a.get,s=a.set;return Object.defineProperty(e,t,{configurable:!0,get:function(){return r.call(this)},set:function(i){n=""+i,s.call(this,i)}}),Object.defineProperty(e,t,{enumerable:a.enumerable}),{getValue:function(){return n},setValue:function(i){n=""+i},stopTracking:function(){e._valueTracker=null,delete e[t]}}}}function gu(e){e._valueTracker||(e._valueTracker=ZR(e))}function by(e){if(!e)return!1;var t=e._valueTracker;if(!t)return!0;var a=t.getValue(),n="";return e&&(n=yy(e)?e.checked?"true":"false":e.value),e=n,e!==a?(t.setValue(e),!0):!1}function yu(e){if(e=e||(typeof document<"u"?document:void 0),typeof e>"u")return null;try{return e.activeElement||e.body}catch{return e.body}}var WR=/[\n"\\]/g;function ga(e){return e.replace(WR,function(t){return"\\"+t.charCodeAt(0).toString(16)+" "})}function km(e,t,a,n,r,s,i,o){e.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?e.type=i:e.removeAttribute("type"),t!=null?i==="number"?(t===0&&e.value===""||e.value!=t)&&(e.value=""+pa(t)):e.value!==""+pa(t)&&(e.value=""+pa(t)):i!=="submit"&&i!=="reset"||e.removeAttribute("value"),t!=null?Rm(e,i,pa(t)):a!=null?Rm(e,i,pa(a)):n!=null&&e.removeAttribute("value"),r==null&&s!=null&&(e.defaultChecked=!!s),r!=null&&(e.checked=r&&typeof r!="function"&&typeof r!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?e.name=""+pa(o):e.removeAttribute("name")}function xy(e,t,a,n,r,s,i,o){if(s!=null&&typeof s!="function"&&typeof s!="symbol"&&typeof s!="boolean"&&(e.type=s),t!=null||a!=null){if(!(s!=="submit"&&s!=="reset"||t!=null))return;a=a!=null?""+pa(a):"",t=t!=null?""+pa(t):a,o||t===e.value||(e.value=t),e.defaultValue=t}n=n??r,n=typeof n!="function"&&typeof n!="symbol"&&!!n,e.checked=o?e.checked:!!n,e.defaultChecked=!!n,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(e.name=i)}function Rm(e,t,a){t==="number"&&yu(e.ownerDocument)===e||e.defaultValue===""+a||(e.defaultValue=""+a)}function $s(e,t,a,n){if(e=e.options,t){t={};for(var r=0;r<a.length;r++)t["$"+a[r]]=!0;for(a=0;a<e.length;a++)r=t.hasOwnProperty("$"+e[a].value),e[a].selected!==r&&(e[a].selected=r),r&&n&&(e[a].defaultSelected=!0)}else{for(a=""+pa(a),t=null,r=0;r<e.length;r++){if(e[r].value===a){e[r].selected=!0,n&&(e[r].defaultSelected=!0);return}t!==null||e[r].disabled||(t=e[r])}t!==null&&(t.selected=!0)}}function $y(e,t,a){if(t!=null&&(t=""+pa(t),t!==e.value&&(e.value=t),a==null)){e.defaultValue!==t&&(e.defaultValue=t);return}e.defaultValue=a!=null?""+pa(a):""}function wy(e,t,a,n){if(t==null){if(n!=null){if(a!=null)throw Error(j(92));if(Qi(n)){if(1<n.length)throw Error(j(93));n=n[0]}a=n}a==null&&(a=""),t=a}a=pa(t),e.defaultValue=a,n=e.textContent,n===a&&n!==""&&n!==null&&(e.value=n)}function Ts(e,t){if(t){var a=e.firstChild;if(a&&a===e.lastChild&&a.nodeType===3){a.nodeValue=t;return}}e.textContent=t}var eC=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function qv(e,t,a){var n=t.indexOf("--")===0;a==null||typeof a=="boolean"||a===""?n?e.setProperty(t,""):t==="float"?e.cssFloat="":e[t]="":n?e.setProperty(t,a):typeof a!="number"||a===0||eC.has(t)?t==="float"?e.cssFloat=a:e[t]=(""+a).trim():e[t]=a+"px"}function Sy(e,t,a){if(t!=null&&typeof t!="object")throw Error(j(62));if(e=e.style,a!=null){for(var n in a)!a.hasOwnProperty(n)||t!=null&&t.hasOwnProperty(n)||(n.indexOf("--")===0?e.setProperty(n,""):n==="float"?e.cssFloat="":e[n]="");for(var r in t)n=t[r],t.hasOwnProperty(r)&&a[r]!==n&&qv(e,r,n)}else for(var s in t)t.hasOwnProperty(s)&&qv(e,s,t[s])}function bf(e){if(e.indexOf("-")===-1)return!1;switch(e){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var tC=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),aC=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function tu(e){return aC.test(""+e)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":e}var Cm=null;function xf(e){return e=e.target||e.srcElement||window,e.correspondingUseElement&&(e=e.correspondingUseElement),e.nodeType===3?e.parentNode:e}var ds=null,ws=null;function Iv(e){var t=Bs(e);if(t&&(e=t.stateNode)){var a=e[qt]||null;e:switch(e=t.stateNode,t.type){case"input":if(km(e,a.value,a.defaultValue,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name),t=a.name,a.type==="radio"&&t!=null){for(a=e;a.parentNode;)a=a.parentNode;for(a=a.querySelectorAll('input[name="'+ga(""+t)+'"][type="radio"]'),t=0;t<a.length;t++){var n=a[t];if(n!==e&&n.form===e.form){var r=n[qt]||null;if(!r)throw Error(j(90));km(n,r.value,r.defaultValue,r.defaultValue,r.checked,r.defaultChecked,r.type,r.name)}}for(t=0;t<a.length;t++)n=a[t],n.form===e.form&&by(n)}break e;case"textarea":$y(e,a.value,a.defaultValue);break e;case"select":t=a.value,t!=null&&$s(e,!!a.multiple,t,!1)}}}var Hd=!1;function Ny(e,t,a){if(Hd)return e(t,a);Hd=!0;try{var n=e(t);return n}finally{if(Hd=!1,(ds!==null||ws!==null)&&(Wu(),ds&&(t=ds,e=ws,ws=ds=null,Iv(t),e)))for(t=0;t<e.length;t++)Iv(e[t])}}function co(e,t){var a=e.stateNode;if(a===null)return null;var n=a[qt]||null;if(n===null)return null;a=n[t];e:switch(t){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(n=!n.disabled)||(e=e.type,n=!(e==="button"||e==="input"||e==="select"||e==="textarea")),e=!n;break e;default:e=!1}if(e)return null;if(a&&typeof a!="function")throw Error(j(231,t,typeof a));return a}var hn=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),Em=!1;if(hn)try{as={},Object.defineProperty(as,"passive",{get:function(){Em=!0}}),window.addEventListener("test",as,as),window.removeEventListener("test",as,as)}catch{Em=!1}var as,In=null,$f=null,au=null;function _y(){if(au)return au;var e,t=$f,a=t.length,n,r="value"in In?In.value:In.textContent,s=r.length;for(e=0;e<a&&t[e]===r[e];e++);var i=a-e;for(n=1;n<=i&&t[a-n]===r[s-n];n++);return au=r.slice(e,1<n?1-n:void 0)}function nu(e){var t=e.keyCode;return"charCode"in e?(e=e.charCode,e===0&&t===13&&(e=13)):e=t,e===10&&(e=13),32<=e||e===13?e:0}function zl(){return!0}function Kv(){return!1}function It(e){function t(a,n,r,s,i){this._reactName=a,this._targetInst=r,this.type=n,this.nativeEvent=s,this.target=i,this.currentTarget=null;for(var o in e)e.hasOwnProperty(o)&&(a=e[o],this[o]=a?a(s):s[o]);return this.isDefaultPrevented=(s.defaultPrevented!=null?s.defaultPrevented:s.returnValue===!1)?zl:Kv,this.isPropagationStopped=Kv,this}return Ae(t.prototype,{preventDefault:function(){this.defaultPrevented=!0;var a=this.nativeEvent;a&&(a.preventDefault?a.preventDefault():typeof a.returnValue!="unknown"&&(a.returnValue=!1),this.isDefaultPrevented=zl)},stopPropagation:function(){var a=this.nativeEvent;a&&(a.stopPropagation?a.stopPropagation():typeof a.cancelBubble!="unknown"&&(a.cancelBubble=!0),this.isPropagationStopped=zl)},persist:function(){},isPersistent:zl}),t}var Dr={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(e){return e.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},Ku=It(Dr),Eo=Ae({},Dr,{view:0,detail:0}),nC=It(Eo),Qd,Vd,ji,Hu=Ae({},Eo,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:wf,button:0,buttons:0,relatedTarget:function(e){return e.relatedTarget===void 0?e.fromElement===e.srcElement?e.toElement:e.fromElement:e.relatedTarget},movementX:function(e){return"movementX"in e?e.movementX:(e!==ji&&(ji&&e.type==="mousemove"?(Qd=e.screenX-ji.screenX,Vd=e.screenY-ji.screenY):Vd=Qd=0,ji=e),Qd)},movementY:function(e){return"movementY"in e?e.movementY:Vd}}),Hv=It(Hu),rC=Ae({},Hu,{dataTransfer:0}),sC=It(rC),iC=Ae({},Eo,{relatedTarget:0}),Gd=It(iC),oC=Ae({},Dr,{animationName:0,elapsedTime:0,pseudoElement:0}),lC=It(oC),uC=Ae({},Dr,{clipboardData:function(e){return"clipboardData"in e?e.clipboardData:window.clipboardData}}),cC=It(uC),dC=Ae({},Dr,{data:0}),Qv=It(dC),mC={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},fC={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},pC={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function hC(e){var t=this.nativeEvent;return t.getModifierState?t.getModifierState(e):(e=pC[e])?!!t[e]:!1}function wf(){return hC}var vC=Ae({},Eo,{key:function(e){if(e.key){var t=mC[e.key]||e.key;if(t!=="Unidentified")return t}return e.type==="keypress"?(e=nu(e),e===13?"Enter":String.fromCharCode(e)):e.type==="keydown"||e.type==="keyup"?fC[e.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:wf,charCode:function(e){return e.type==="keypress"?nu(e):0},keyCode:function(e){return e.type==="keydown"||e.type==="keyup"?e.keyCode:0},which:function(e){return e.type==="keypress"?nu(e):e.type==="keydown"||e.type==="keyup"?e.keyCode:0}}),gC=It(vC),yC=Ae({},Hu,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Vv=It(yC),bC=Ae({},Eo,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:wf}),xC=It(bC),$C=Ae({},Dr,{propertyName:0,elapsedTime:0,pseudoElement:0}),wC=It($C),SC=Ae({},Hu,{deltaX:function(e){return"deltaX"in e?e.deltaX:"wheelDeltaX"in e?-e.wheelDeltaX:0},deltaY:function(e){return"deltaY"in e?e.deltaY:"wheelDeltaY"in e?-e.wheelDeltaY:"wheelDelta"in e?-e.wheelDelta:0},deltaZ:0,deltaMode:0}),NC=It(SC),_C=Ae({},Dr,{newState:0,oldState:0}),kC=It(_C),RC=[9,13,27,32],Sf=hn&&"CompositionEvent"in window,Yi=null;hn&&"documentMode"in document&&(Yi=document.documentMode);var CC=hn&&"TextEvent"in window&&!Yi,ky=hn&&(!Sf||Yi&&8<Yi&&11>=Yi),Gv=" ",Yv=!1;function Ry(e,t){switch(e){case"keyup":return RC.indexOf(t.keyCode)!==-1;case"keydown":return t.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function Cy(e){return e=e.detail,typeof e=="object"&&"data"in e?e.data:null}var ms=!1;function EC(e,t){switch(e){case"compositionend":return Cy(t);case"keypress":return t.which!==32?null:(Yv=!0,Gv);case"textInput":return e=t.data,e===Gv&&Yv?null:e;default:return null}}function TC(e,t){if(ms)return e==="compositionend"||!Sf&&Ry(e,t)?(e=_y(),au=$f=In=null,ms=!1,e):null;switch(e){case"paste":return null;case"keypress":if(!(t.ctrlKey||t.altKey||t.metaKey)||t.ctrlKey&&t.altKey){if(t.char&&1<t.char.length)return t.char;if(t.which)return String.fromCharCode(t.which)}return null;case"compositionend":return ky&&t.locale!=="ko"?null:t.data;default:return null}}var AC={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function Jv(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t==="input"?!!AC[e.type]:t==="textarea"}function Ey(e,t,a,n){ds?ws?ws.push(n):ws=[n]:ds=n,t=Pu(t,"onChange"),0<t.length&&(a=new Ku("onChange","change",null,a,n),e.push({event:a,listeners:t}))}var Ji=null,mo=null;function DC(e){$0(e,0)}function Qu(e){var t=Vi(e);if(by(t))return e}function Xv(e,t){if(e==="change")return t}var Ty=!1;hn&&(hn?(Il="oninput"in document,Il||(Yd=document.createElement("div"),Yd.setAttribute("oninput","return;"),Il=typeof Yd.oninput=="function"),ql=Il):ql=!1,Ty=ql&&(!document.documentMode||9<document.documentMode));var ql,Il,Yd;function Zv(){Ji&&(Ji.detachEvent("onpropertychange",Ay),mo=Ji=null)}function Ay(e){if(e.propertyName==="value"&&Qu(mo)){var t=[];Ey(t,mo,e,xf(e)),Ny(DC,t)}}function MC(e,t,a){e==="focusin"?(Zv(),Ji=t,mo=a,Ji.attachEvent("onpropertychange",Ay)):e==="focusout"&&Zv()}function OC(e){if(e==="selectionchange"||e==="keyup"||e==="keydown")return Qu(mo)}function LC(e,t){if(e==="click")return Qu(t)}function PC(e,t){if(e==="input"||e==="change")return Qu(t)}function UC(e,t){return e===t&&(e!==0||1/e===1/t)||e!==e&&t!==t}var aa=typeof Object.is=="function"?Object.is:UC;function fo(e,t){if(aa(e,t))return!0;if(typeof e!="object"||e===null||typeof t!="object"||t===null)return!1;var a=Object.keys(e),n=Object.keys(t);if(a.length!==n.length)return!1;for(n=0;n<a.length;n++){var r=a[n];if(!Nm.call(t,r)||!aa(e[r],t[r]))return!1}return!0}function Wv(e){for(;e&&e.firstChild;)e=e.firstChild;return e}function eg(e,t){var a=Wv(e);e=0;for(var n;a;){if(a.nodeType===3){if(n=e+a.textContent.length,e<=t&&n>=t)return{node:a,offset:t-e};e=n}e:{for(;a;){if(a.nextSibling){a=a.nextSibling;break e}a=a.parentNode}a=void 0}a=Wv(a)}}function Dy(e,t){return e&&t?e===t?!0:e&&e.nodeType===3?!1:t&&t.nodeType===3?Dy(e,t.parentNode):"contains"in e?e.contains(t):e.compareDocumentPosition?!!(e.compareDocumentPosition(t)&16):!1:!1}function My(e){e=e!=null&&e.ownerDocument!=null&&e.ownerDocument.defaultView!=null?e.ownerDocument.defaultView:window;for(var t=yu(e.document);t instanceof e.HTMLIFrameElement;){try{var a=typeof t.contentWindow.location.href=="string"}catch{a=!1}if(a)e=t.contentWindow;else break;t=yu(e.document)}return t}function Nf(e){var t=e&&e.nodeName&&e.nodeName.toLowerCase();return t&&(t==="input"&&(e.type==="text"||e.type==="search"||e.type==="tel"||e.type==="url"||e.type==="password")||t==="textarea"||e.contentEditable==="true")}var jC=hn&&"documentMode"in document&&11>=document.documentMode,fs=null,Tm=null,Xi=null,Am=!1;function tg(e,t,a){var n=a.window===a?a.document:a.nodeType===9?a:a.ownerDocument;Am||fs==null||fs!==yu(n)||(n=fs,"selectionStart"in n&&Nf(n)?n={start:n.selectionStart,end:n.selectionEnd}:(n=(n.ownerDocument&&n.ownerDocument.defaultView||window).getSelection(),n={anchorNode:n.anchorNode,anchorOffset:n.anchorOffset,focusNode:n.focusNode,focusOffset:n.focusOffset}),Xi&&fo(Xi,n)||(Xi=n,n=Pu(Tm,"onSelect"),0<n.length&&(t=new Ku("onSelect","select",null,t,a),e.push({event:t,listeners:n}),t.target=fs)))}function vr(e,t){var a={};return a[e.toLowerCase()]=t.toLowerCase(),a["Webkit"+e]="webkit"+t,a["Moz"+e]="moz"+t,a}var ps={animationend:vr("Animation","AnimationEnd"),animationiteration:vr("Animation","AnimationIteration"),animationstart:vr("Animation","AnimationStart"),transitionrun:vr("Transition","TransitionRun"),transitionstart:vr("Transition","TransitionStart"),transitioncancel:vr("Transition","TransitionCancel"),transitionend:vr("Transition","TransitionEnd")},Jd={},Oy={};hn&&(Oy=document.createElement("div").style,"AnimationEvent"in window||(delete ps.animationend.animation,delete ps.animationiteration.animation,delete ps.animationstart.animation),"TransitionEvent"in window||delete ps.transitionend.transition);function Mr(e){if(Jd[e])return Jd[e];if(!ps[e])return e;var t=ps[e],a;for(a in t)if(t.hasOwnProperty(a)&&a in Oy)return Jd[e]=t[a];return e}var Ly=Mr("animationend"),Py=Mr("animationiteration"),Uy=Mr("animationstart"),FC=Mr("transitionrun"),BC=Mr("transitionstart"),zC=Mr("transitioncancel"),jy=Mr("transitionend"),Fy=new Map,Dm="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");Dm.push("scrollEnd");function Ra(e,t){Fy.set(e,t),Ar(t,[e])}var ag=new WeakMap;function ya(e,t){if(typeof e=="object"&&e!==null){var a=ag.get(e);return a!==void 0?a:(t={value:e,source:t,stack:zv(t)},ag.set(e,t),t)}return{value:e,source:t,stack:zv(t)}}var fa=[],hs=0,_f=0;function Vu(){for(var e=hs,t=_f=hs=0;t<e;){var a=fa[t];fa[t++]=null;var n=fa[t];fa[t++]=null;var r=fa[t];fa[t++]=null;var s=fa[t];if(fa[t++]=null,n!==null&&r!==null){var i=n.pending;i===null?r.next=r:(r.next=i.next,i.next=r),n.pending=r}s!==0&&By(a,r,s)}}function Gu(e,t,a,n){fa[hs++]=e,fa[hs++]=t,fa[hs++]=a,fa[hs++]=n,_f|=n,e.lanes|=n,e=e.alternate,e!==null&&(e.lanes|=n)}function kf(e,t,a,n){return Gu(e,t,a,n),bu(e)}function zs(e,t){return Gu(e,null,null,t),bu(e)}function By(e,t,a){e.lanes|=a;var n=e.alternate;n!==null&&(n.lanes|=a);for(var r=!1,s=e.return;s!==null;)s.childLanes|=a,n=s.alternate,n!==null&&(n.childLanes|=a),s.tag===22&&(e=s.stateNode,e===null||e._visibility&1||(r=!0)),e=s,s=s.return;return e.tag===3?(s=e.stateNode,r&&t!==null&&(r=31-Wt(a),e=s.hiddenUpdates,n=e[r],n===null?e[r]=[t]:n.push(t),t.lane=a|536870912),s):null}function bu(e){if(50<oo)throw oo=0,Wm=null,Error(j(185));for(var t=e.return;t!==null;)e=t,t=e.return;return e.tag===3?e.stateNode:null}var vs={};function qC(e,t,a,n){this.tag=e,this.key=a,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=t,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=n,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Xt(e,t,a,n){return new qC(e,t,a,n)}function Rf(e){return e=e.prototype,!(!e||!e.isReactComponent)}function fn(e,t){var a=e.alternate;return a===null?(a=Xt(e.tag,t,e.key,e.mode),a.elementType=e.elementType,a.type=e.type,a.stateNode=e.stateNode,a.alternate=e,e.alternate=a):(a.pendingProps=t,a.type=e.type,a.flags=0,a.subtreeFlags=0,a.deletions=null),a.flags=e.flags&65011712,a.childLanes=e.childLanes,a.lanes=e.lanes,a.child=e.child,a.memoizedProps=e.memoizedProps,a.memoizedState=e.memoizedState,a.updateQueue=e.updateQueue,t=e.dependencies,a.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext},a.sibling=e.sibling,a.index=e.index,a.ref=e.ref,a.refCleanup=e.refCleanup,a}function zy(e,t){e.flags&=65011714;var a=e.alternate;return a===null?(e.childLanes=0,e.lanes=t,e.child=null,e.subtreeFlags=0,e.memoizedProps=null,e.memoizedState=null,e.updateQueue=null,e.dependencies=null,e.stateNode=null):(e.childLanes=a.childLanes,e.lanes=a.lanes,e.child=a.child,e.subtreeFlags=0,e.deletions=null,e.memoizedProps=a.memoizedProps,e.memoizedState=a.memoizedState,e.updateQueue=a.updateQueue,e.type=a.type,t=a.dependencies,e.dependencies=t===null?null:{lanes:t.lanes,firstContext:t.firstContext}),e}function ru(e,t,a,n,r,s){var i=0;if(n=e,typeof e=="function")Rf(e)&&(i=1);else if(typeof e=="string")i=qE(e,a,Ba.current)?26:e==="html"||e==="head"||e==="body"?27:5;else e:switch(e){case xm:return e=Xt(31,a,t,r),e.elementType=xm,e.lanes=s,e;case ls:return $r(a.children,r,s,t);case iy:i=8,r|=24;break;case gm:return e=Xt(12,a,t,r|2),e.elementType=gm,e.lanes=s,e;case ym:return e=Xt(13,a,t,r),e.elementType=ym,e.lanes=s,e;case bm:return e=Xt(19,a,t,r),e.elementType=bm,e.lanes=s,e;default:if(typeof e=="object"&&e!==null)switch(e.$$typeof){case DR:case ln:i=10;break e;case oy:i=9;break e;case ff:i=11;break e;case pf:i=14;break e;case Pn:i=16,n=null;break e}i=29,a=Error(j(130,e===null?"null":typeof e,"")),n=null}return t=Xt(i,a,t,r),t.elementType=e,t.type=n,t.lanes=s,t}function $r(e,t,a,n){return e=Xt(7,e,n,t),e.lanes=a,e}function Xd(e,t,a){return e=Xt(6,e,null,t),e.lanes=a,e}function Zd(e,t,a){return t=Xt(4,e.children!==null?e.children:[],e.key,t),t.lanes=a,t.stateNode={containerInfo:e.containerInfo,pendingChildren:null,implementation:e.implementation},t}var gs=[],ys=0,xu=null,$u=0,ha=[],va=0,wr=null,un=1,cn="";function yr(e,t){gs[ys++]=$u,gs[ys++]=xu,xu=e,$u=t}function qy(e,t,a){ha[va++]=un,ha[va++]=cn,ha[va++]=wr,wr=e;var n=un;e=cn;var r=32-Wt(n)-1;n&=~(1<<r),a+=1;var s=32-Wt(t)+r;if(30<s){var i=r-r%5;s=(n&(1<<i)-1).toString(32),n>>=i,r-=i,un=1<<32-Wt(t)+r|a<<r|n,cn=s+e}else un=1<<s|a<<r|n,cn=e}function Cf(e){e.return!==null&&(yr(e,1),qy(e,1,0))}function Ef(e){for(;e===xu;)xu=gs[--ys],gs[ys]=null,$u=gs[--ys],gs[ys]=null;for(;e===wr;)wr=ha[--va],ha[va]=null,cn=ha[--va],ha[va]=null,un=ha[--va],ha[va]=null}var Tt=null,qe=null,ve=!1,Sr=null,ja=!1,Mm=Error(j(519));function Rr(e){var t=Error(j(418,""));throw po(ya(t,e)),Mm}function ng(e){var t=e.stateNode,a=e.type,n=e.memoizedProps;switch(t[Nt]=e,t[qt]=n,a){case"dialog":le("cancel",t),le("close",t);break;case"iframe":case"object":case"embed":le("load",t);break;case"video":case"audio":for(a=0;a<go.length;a++)le(go[a],t);break;case"source":le("error",t);break;case"img":case"image":case"link":le("error",t),le("load",t);break;case"details":le("toggle",t);break;case"input":le("invalid",t),xy(t,n.value,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name,!0),gu(t);break;case"select":le("invalid",t);break;case"textarea":le("invalid",t),wy(t,n.value,n.defaultValue,n.children),gu(t)}a=n.children,typeof a!="string"&&typeof a!="number"&&typeof a!="bigint"||t.textContent===""+a||n.suppressHydrationWarning===!0||S0(t.textContent,a)?(n.popover!=null&&(le("beforetoggle",t),le("toggle",t)),n.onScroll!=null&&le("scroll",t),n.onScrollEnd!=null&&le("scrollend",t),n.onClick!=null&&(t.onclick=ac),t=!0):t=!1,t||Rr(e)}function rg(e){for(Tt=e.return;Tt;)switch(Tt.tag){case 5:case 13:ja=!1;return;case 27:case 3:ja=!0;return;default:Tt=Tt.return}}function Fi(e){if(e!==Tt)return!1;if(!ve)return rg(e),ve=!0,!1;var t=e.tag,a;if((a=t!==3&&t!==27)&&((a=t===5)&&(a=e.type,a=!(a!=="form"&&a!=="button")||sf(e.type,e.memoizedProps)),a=!a),a&&qe&&Rr(e),rg(e),t===13){if(e=e.memoizedState,e=e!==null?e.dehydrated:null,!e)throw Error(j(317));e:{for(e=e.nextSibling,t=0;e;){if(e.nodeType===8)if(a=e.data,a==="/$"){if(t===0){qe=ka(e.nextSibling);break e}t--}else a!=="$"&&a!=="$!"&&a!=="$?"||t++;e=e.nextSibling}qe=null}}else t===27?(t=qe,rr(e.type)?(e=uf,uf=null,qe=e):qe=t):qe=Tt?ka(e.stateNode.nextSibling):null;return!0}function To(){qe=Tt=null,ve=!1}function sg(){var e=Sr;return e!==null&&(zt===null?zt=e:zt.push.apply(zt,e),Sr=null),e}function po(e){Sr===null?Sr=[e]:Sr.push(e)}var Om=Ia(null),Or=null,dn=null;function jn(e,t,a){Le(Om,t._currentValue),t._currentValue=a}function pn(e){e._currentValue=Om.current,mt(Om)}function Lm(e,t,a){for(;e!==null;){var n=e.alternate;if((e.childLanes&t)!==t?(e.childLanes|=t,n!==null&&(n.childLanes|=t)):n!==null&&(n.childLanes&t)!==t&&(n.childLanes|=t),e===a)break;e=e.return}}function Pm(e,t,a,n){var r=e.child;for(r!==null&&(r.return=e);r!==null;){var s=r.dependencies;if(s!==null){var i=r.child;s=s.firstContext;e:for(;s!==null;){var o=s;s=r;for(var u=0;u<t.length;u++)if(o.context===t[u]){s.lanes|=a,o=s.alternate,o!==null&&(o.lanes|=a),Lm(s.return,a,e),n||(i=null);break e}s=o.next}}else if(r.tag===18){if(i=r.return,i===null)throw Error(j(341));i.lanes|=a,s=i.alternate,s!==null&&(s.lanes|=a),Lm(i,a,e),i=null}else i=r.child;if(i!==null)i.return=r;else for(i=r;i!==null;){if(i===e){i=null;break}if(r=i.sibling,r!==null){r.return=i.return,i=r;break}i=i.return}r=i}}function Ao(e,t,a,n){e=null;for(var r=t,s=!1;r!==null;){if(!s){if((r.flags&524288)!==0)s=!0;else if((r.flags&262144)!==0)break}if(r.tag===10){var i=r.alternate;if(i===null)throw Error(j(387));if(i=i.memoizedProps,i!==null){var o=r.type;aa(r.pendingProps.value,i.value)||(e!==null?e.push(o):e=[o])}}else if(r===fu.current){if(i=r.alternate,i===null)throw Error(j(387));i.memoizedState.memoizedState!==r.memoizedState.memoizedState&&(e!==null?e.push(xo):e=[xo])}r=r.return}e!==null&&Pm(t,e,a,n),t.flags|=262144}function wu(e){for(e=e.firstContext;e!==null;){if(!aa(e.context._currentValue,e.memoizedValue))return!0;e=e.next}return!1}function Cr(e){Or=e,dn=null,e=e.dependencies,e!==null&&(e.firstContext=null)}function _t(e){return Iy(Or,e)}function Kl(e,t){return Or===null&&Cr(e),Iy(e,t)}function Iy(e,t){var a=t._currentValue;if(t={context:t,memoizedValue:a,next:null},dn===null){if(e===null)throw Error(j(308));dn=t,e.dependencies={lanes:0,firstContext:t},e.flags|=524288}else dn=dn.next=t;return a}var IC=typeof AbortController<"u"?AbortController:function(){var e=[],t=this.signal={aborted:!1,addEventListener:function(a,n){e.push(n)}};this.abort=function(){t.aborted=!0,e.forEach(function(a){return a()})}},KC=st.unstable_scheduleCallback,HC=st.unstable_NormalPriority,nt={$$typeof:ln,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function Tf(){return{controller:new IC,data:new Map,refCount:0}}function Do(e){e.refCount--,e.refCount===0&&KC(HC,function(){e.controller.abort()})}var Zi=null,Um=0,As=0,Ss=null;function QC(e,t){if(Zi===null){var a=Zi=[];Um=0,As=Wf(),Ss={status:"pending",value:void 0,then:function(n){a.push(n)}}}return Um++,t.then(ig,ig),t}function ig(){if(--Um===0&&Zi!==null){Ss!==null&&(Ss.status="fulfilled");var e=Zi;Zi=null,As=0,Ss=null;for(var t=0;t<e.length;t++)(0,e[t])()}}function VC(e,t){var a=[],n={status:"pending",value:null,reason:null,then:function(r){a.push(r)}};return e.then(function(){n.status="fulfilled",n.value=t;for(var r=0;r<a.length;r++)(0,a[r])(t)},function(r){for(n.status="rejected",n.reason=r,r=0;r<a.length;r++)(0,a[r])(void 0)}),n}var og=ne.S;ne.S=function(e,t){typeof t=="object"&&t!==null&&typeof t.then=="function"&&QC(e,t),og!==null&&og(e,t)};var Nr=Ia(null);function Af(){var e=Nr.current;return e!==null?e:ke.pooledCache}function su(e,t){t===null?Le(Nr,Nr.current):Le(Nr,t.pool)}function Ky(){var e=Af();return e===null?null:{parent:nt._currentValue,pool:e}}var Mo=Error(j(460)),Hy=Error(j(474)),Yu=Error(j(542)),jm={then:function(){}};function lg(e){return e=e.status,e==="fulfilled"||e==="rejected"}function Hl(){}function Qy(e,t,a){switch(a=e[a],a===void 0?e.push(t):a!==t&&(t.then(Hl,Hl),t=a),t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,cg(e),e;default:if(typeof t.status=="string")t.then(Hl,Hl);else{if(e=ke,e!==null&&100<e.shellSuspendCounter)throw Error(j(482));e=t,e.status="pending",e.then(function(n){if(t.status==="pending"){var r=t;r.status="fulfilled",r.value=n}},function(n){if(t.status==="pending"){var r=t;r.status="rejected",r.reason=n}})}switch(t.status){case"fulfilled":return t.value;case"rejected":throw e=t.reason,cg(e),e}throw Wi=t,Mo}}var Wi=null;function ug(){if(Wi===null)throw Error(j(459));var e=Wi;return Wi=null,e}function cg(e){if(e===Mo||e===Yu)throw Error(j(483))}var Un=!1;function Df(e){e.updateQueue={baseState:e.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function Fm(e,t){e=e.updateQueue,t.updateQueue===e&&(t.updateQueue={baseState:e.baseState,firstBaseUpdate:e.firstBaseUpdate,lastBaseUpdate:e.lastBaseUpdate,shared:e.shared,callbacks:null})}function Vn(e){return{lane:e,tag:0,payload:null,callback:null,next:null}}function Gn(e,t,a){var n=e.updateQueue;if(n===null)return null;if(n=n.shared,(we&2)!==0){var r=n.pending;return r===null?t.next=t:(t.next=r.next,r.next=t),n.pending=t,t=bu(e),By(e,null,a),t}return Gu(e,n,t,a),bu(e)}function eo(e,t,a){if(t=t.updateQueue,t!==null&&(t=t.shared,(a&4194048)!==0)){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,py(e,a)}}function Wd(e,t){var a=e.updateQueue,n=e.alternate;if(n!==null&&(n=n.updateQueue,a===n)){var r=null,s=null;if(a=a.firstBaseUpdate,a!==null){do{var i={lane:a.lane,tag:a.tag,payload:a.payload,callback:null,next:null};s===null?r=s=i:s=s.next=i,a=a.next}while(a!==null);s===null?r=s=t:s=s.next=t}else r=s=t;a={baseState:n.baseState,firstBaseUpdate:r,lastBaseUpdate:s,shared:n.shared,callbacks:n.callbacks},e.updateQueue=a;return}e=a.lastBaseUpdate,e===null?a.firstBaseUpdate=t:e.next=t,a.lastBaseUpdate=t}var Bm=!1;function to(){if(Bm){var e=Ss;if(e!==null)throw e}}function ao(e,t,a,n){Bm=!1;var r=e.updateQueue;Un=!1;var s=r.firstBaseUpdate,i=r.lastBaseUpdate,o=r.shared.pending;if(o!==null){r.shared.pending=null;var u=o,c=u.next;u.next=null,i===null?s=c:i.next=c,i=u;var d=e.alternate;d!==null&&(d=d.updateQueue,o=d.lastBaseUpdate,o!==i&&(o===null?d.firstBaseUpdate=c:o.next=c,d.lastBaseUpdate=u))}if(s!==null){var f=r.baseState;i=0,d=c=u=null,o=s;do{var m=o.lane&-536870913,h=m!==o.lane;if(h?(de&m)===m:(n&m)===m){m!==0&&m===As&&(Bm=!0),d!==null&&(d=d.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});e:{var b=e,y=o;m=t;var w=a;switch(y.tag){case 1:if(b=y.payload,typeof b=="function"){f=b.call(w,f,m);break e}f=b;break e;case 3:b.flags=b.flags&-65537|128;case 0:if(b=y.payload,m=typeof b=="function"?b.call(w,f,m):b,m==null)break e;f=Ae({},f,m);break e;case 2:Un=!0}}m=o.callback,m!==null&&(e.flags|=64,h&&(e.flags|=8192),h=r.callbacks,h===null?r.callbacks=[m]:h.push(m))}else h={lane:m,tag:o.tag,payload:o.payload,callback:o.callback,next:null},d===null?(c=d=h,u=f):d=d.next=h,i|=m;if(o=o.next,o===null){if(o=r.shared.pending,o===null)break;h=o,o=h.next,h.next=null,r.lastBaseUpdate=h,r.shared.pending=null}}while(!0);d===null&&(u=f),r.baseState=u,r.firstBaseUpdate=c,r.lastBaseUpdate=d,s===null&&(r.shared.lanes=0),ar|=i,e.lanes=i,e.memoizedState=f}}function Vy(e,t){if(typeof e!="function")throw Error(j(191,e));e.call(t)}function Gy(e,t){var a=e.callbacks;if(a!==null)for(e.callbacks=null,e=0;e<a.length;e++)Vy(a[e],t)}var Ds=Ia(null),Su=Ia(0);function dg(e,t){e=yn,Le(Su,e),Le(Ds,t),yn=e|t.baseLanes}function zm(){Le(Su,yn),Le(Ds,Ds.current)}function Mf(){yn=Su.current,mt(Ds),mt(Su)}var er=0,se=null,Ne=null,Je=null,Nu=!1,Ns=!1,Er=!1,_u=0,ho=0,_s=null,GC=0;function Qe(){throw Error(j(321))}function Of(e,t){if(t===null)return!1;for(var a=0;a<t.length&&a<e.length;a++)if(!aa(e[a],t[a]))return!1;return!0}function Lf(e,t,a,n,r,s){return er=s,se=t,t.memoizedState=null,t.updateQueue=null,t.lanes=0,ne.H=e===null||e.memoizedState===null?_b:kb,Er=!1,s=a(n,r),Er=!1,Ns&&(s=Jy(t,a,n,r)),Yy(e),s}function Yy(e){ne.H=ku;var t=Ne!==null&&Ne.next!==null;if(er=0,Je=Ne=se=null,Nu=!1,ho=0,_s=null,t)throw Error(j(300));e===null||dt||(e=e.dependencies,e!==null&&wu(e)&&(dt=!0))}function Jy(e,t,a,n){se=e;var r=0;do{if(Ns&&(_s=null),ho=0,Ns=!1,25<=r)throw Error(j(301));if(r+=1,Je=Ne=null,e.updateQueue!=null){var s=e.updateQueue;s.lastEffect=null,s.events=null,s.stores=null,s.memoCache!=null&&(s.memoCache.index=0)}ne.H=tE,s=t(a,n)}while(Ns);return s}function YC(){var e=ne.H,t=e.useState()[0];return t=typeof t.then=="function"?Oo(t):t,e=e.useState()[0],(Ne!==null?Ne.memoizedState:null)!==e&&(se.flags|=1024),t}function Pf(){var e=_u!==0;return _u=0,e}function Uf(e,t,a){t.updateQueue=e.updateQueue,t.flags&=-2053,e.lanes&=~a}function jf(e){if(Nu){for(e=e.memoizedState;e!==null;){var t=e.queue;t!==null&&(t.pending=null),e=e.next}Nu=!1}er=0,Je=Ne=se=null,Ns=!1,ho=_u=0,_s=null}function Ft(){var e={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return Je===null?se.memoizedState=Je=e:Je=Je.next=e,Je}function Xe(){if(Ne===null){var e=se.alternate;e=e!==null?e.memoizedState:null}else e=Ne.next;var t=Je===null?se.memoizedState:Je.next;if(t!==null)Je=t,Ne=e;else{if(e===null)throw se.alternate===null?Error(j(467)):Error(j(310));Ne=e,e={memoizedState:Ne.memoizedState,baseState:Ne.baseState,baseQueue:Ne.baseQueue,queue:Ne.queue,next:null},Je===null?se.memoizedState=Je=e:Je=Je.next=e}return Je}function Ff(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Oo(e){var t=ho;return ho+=1,_s===null&&(_s=[]),e=Qy(_s,e,t),t=se,(Je===null?t.memoizedState:Je.next)===null&&(t=t.alternate,ne.H=t===null||t.memoizedState===null?_b:kb),e}function Ju(e){if(e!==null&&typeof e=="object"){if(typeof e.then=="function")return Oo(e);if(e.$$typeof===ln)return _t(e)}throw Error(j(438,String(e)))}function Bf(e){var t=null,a=se.updateQueue;if(a!==null&&(t=a.memoCache),t==null){var n=se.alternate;n!==null&&(n=n.updateQueue,n!==null&&(n=n.memoCache,n!=null&&(t={data:n.data.map(function(r){return r.slice()}),index:0})))}if(t==null&&(t={data:[],index:0}),a===null&&(a=Ff(),se.updateQueue=a),a.memoCache=t,a=t.data[t.index],a===void 0)for(a=t.data[t.index]=Array(e),n=0;n<e;n++)a[n]=MR;return t.index++,a}function vn(e,t){return typeof t=="function"?t(e):t}function iu(e){var t=Xe();return zf(t,Ne,e)}function zf(e,t,a){var n=e.queue;if(n===null)throw Error(j(311));n.lastRenderedReducer=a;var r=e.baseQueue,s=n.pending;if(s!==null){if(r!==null){var i=r.next;r.next=s.next,s.next=i}t.baseQueue=r=s,n.pending=null}if(s=e.baseState,r===null)e.memoizedState=s;else{t=r.next;var o=i=null,u=null,c=t,d=!1;do{var f=c.lane&-536870913;if(f!==c.lane?(de&f)===f:(er&f)===f){var m=c.revertLane;if(m===0)u!==null&&(u=u.next={lane:0,revertLane:0,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null}),f===As&&(d=!0);else if((er&m)===m){c=c.next,m===As&&(d=!0);continue}else f={lane:0,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=f,i=s):u=u.next=f,se.lanes|=m,ar|=m;f=c.action,Er&&a(s,f),s=c.hasEagerState?c.eagerState:a(s,f)}else m={lane:f,revertLane:c.revertLane,action:c.action,hasEagerState:c.hasEagerState,eagerState:c.eagerState,next:null},u===null?(o=u=m,i=s):u=u.next=m,se.lanes|=f,ar|=f;c=c.next}while(c!==null&&c!==t);if(u===null?i=s:u.next=o,!aa(s,e.memoizedState)&&(dt=!0,d&&(a=Ss,a!==null)))throw a;e.memoizedState=s,e.baseState=i,e.baseQueue=u,n.lastRenderedState=s}return r===null&&(n.lanes=0),[e.memoizedState,n.dispatch]}function em(e){var t=Xe(),a=t.queue;if(a===null)throw Error(j(311));a.lastRenderedReducer=e;var n=a.dispatch,r=a.pending,s=t.memoizedState;if(r!==null){a.pending=null;var i=r=r.next;do s=e(s,i.action),i=i.next;while(i!==r);aa(s,t.memoizedState)||(dt=!0),t.memoizedState=s,t.baseQueue===null&&(t.baseState=s),a.lastRenderedState=s}return[s,n]}function Xy(e,t,a){var n=se,r=Xe(),s=ve;if(s){if(a===void 0)throw Error(j(407));a=a()}else a=t();var i=!aa((Ne||r).memoizedState,a);i&&(r.memoizedState=a,dt=!0),r=r.queue;var o=eb.bind(null,n,r,e);if(Lo(2048,8,o,[e]),r.getSnapshot!==t||i||Je!==null&&Je.memoizedState.tag&1){if(n.flags|=2048,Ms(9,Xu(),Wy.bind(null,n,r,a,t),null),ke===null)throw Error(j(349));s||(er&124)!==0||Zy(n,t,a)}return a}function Zy(e,t,a){e.flags|=16384,e={getSnapshot:t,value:a},t=se.updateQueue,t===null?(t=Ff(),se.updateQueue=t,t.stores=[e]):(a=t.stores,a===null?t.stores=[e]:a.push(e))}function Wy(e,t,a,n){t.value=a,t.getSnapshot=n,tb(t)&&ab(e)}function eb(e,t,a){return a(function(){tb(t)&&ab(e)})}function tb(e){var t=e.getSnapshot;e=e.value;try{var a=t();return!aa(e,a)}catch{return!0}}function ab(e){var t=zs(e,2);t!==null&&ta(t,e,2)}function qm(e){var t=Ft();if(typeof e=="function"){var a=e;if(e=a(),Er){qn(!0);try{a()}finally{qn(!1)}}}return t.memoizedState=t.baseState=e,t.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:vn,lastRenderedState:e},t}function nb(e,t,a,n){return e.baseState=a,zf(e,Ne,typeof n=="function"?n:vn)}function JC(e,t,a,n,r){if(Zu(e))throw Error(j(485));if(e=t.action,e!==null){var s={payload:r,action:e,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){s.listeners.push(i)}};ne.T!==null?a(!0):s.isTransition=!1,n(s),a=t.pending,a===null?(s.next=t.pending=s,rb(t,s)):(s.next=a.next,t.pending=a.next=s)}}function rb(e,t){var a=t.action,n=t.payload,r=e.state;if(t.isTransition){var s=ne.T,i={};ne.T=i;try{var o=a(r,n),u=ne.S;u!==null&&u(i,o),mg(e,t,o)}catch(c){Im(e,t,c)}finally{ne.T=s}}else try{s=a(r,n),mg(e,t,s)}catch(c){Im(e,t,c)}}function mg(e,t,a){a!==null&&typeof a=="object"&&typeof a.then=="function"?a.then(function(n){fg(e,t,n)},function(n){return Im(e,t,n)}):fg(e,t,a)}function fg(e,t,a){t.status="fulfilled",t.value=a,sb(t),e.state=a,t=e.pending,t!==null&&(a=t.next,a===t?e.pending=null:(a=a.next,t.next=a,rb(e,a)))}function Im(e,t,a){var n=e.pending;if(e.pending=null,n!==null){n=n.next;do t.status="rejected",t.reason=a,sb(t),t=t.next;while(t!==n)}e.action=null}function sb(e){e=e.listeners;for(var t=0;t<e.length;t++)(0,e[t])()}function ib(e,t){return t}function pg(e,t){if(ve){var a=ke.formState;if(a!==null){e:{var n=se;if(ve){if(qe){t:{for(var r=qe,s=ja;r.nodeType!==8;){if(!s){r=null;break t}if(r=ka(r.nextSibling),r===null){r=null;break t}}s=r.data,r=s==="F!"||s==="F"?r:null}if(r){qe=ka(r.nextSibling),n=r.data==="F!";break e}}Rr(n)}n=!1}n&&(t=a[0])}}return a=Ft(),a.memoizedState=a.baseState=t,n={pending:null,lanes:0,dispatch:null,lastRenderedReducer:ib,lastRenderedState:t},a.queue=n,a=wb.bind(null,se,n),n.dispatch=a,n=qm(!1),s=Hf.bind(null,se,!1,n.queue),n=Ft(),r={state:t,dispatch:null,action:e,pending:null},n.queue=r,a=JC.bind(null,se,r,s,a),r.dispatch=a,n.memoizedState=e,[t,a,!1]}function hg(e){var t=Xe();return ob(t,Ne,e)}function ob(e,t,a){if(t=zf(e,t,ib)[0],e=iu(vn)[0],typeof t=="object"&&t!==null&&typeof t.then=="function")try{var n=Oo(t)}catch(i){throw i===Mo?Yu:i}else n=t;t=Xe();var r=t.queue,s=r.dispatch;return a!==t.memoizedState&&(se.flags|=2048,Ms(9,Xu(),XC.bind(null,r,a),null)),[n,s,e]}function XC(e,t){e.action=t}function vg(e){var t=Xe(),a=Ne;if(a!==null)return ob(t,a,e);Xe(),t=t.memoizedState,a=Xe();var n=a.queue.dispatch;return a.memoizedState=e,[t,n,!1]}function Ms(e,t,a,n){return e={tag:e,create:a,deps:n,inst:t,next:null},t=se.updateQueue,t===null&&(t=Ff(),se.updateQueue=t),a=t.lastEffect,a===null?t.lastEffect=e.next=e:(n=a.next,a.next=e,e.next=n,t.lastEffect=e),e}function Xu(){return{destroy:void 0,resource:void 0}}function lb(){return Xe().memoizedState}function ou(e,t,a,n){var r=Ft();n=n===void 0?null:n,se.flags|=e,r.memoizedState=Ms(1|t,Xu(),a,n)}function Lo(e,t,a,n){var r=Xe();n=n===void 0?null:n;var s=r.memoizedState.inst;Ne!==null&&n!==null&&Of(n,Ne.memoizedState.deps)?r.memoizedState=Ms(t,s,a,n):(se.flags|=e,r.memoizedState=Ms(1|t,s,a,n))}function gg(e,t){ou(8390656,8,e,t)}function ub(e,t){Lo(2048,8,e,t)}function cb(e,t){return Lo(4,2,e,t)}function db(e,t){return Lo(4,4,e,t)}function mb(e,t){if(typeof t=="function"){e=e();var a=t(e);return function(){typeof a=="function"?a():t(null)}}if(t!=null)return e=e(),t.current=e,function(){t.current=null}}function fb(e,t,a){a=a!=null?a.concat([e]):null,Lo(4,4,mb.bind(null,t,e),a)}function qf(){}function pb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;return t!==null&&Of(t,n[1])?n[0]:(a.memoizedState=[e,t],e)}function hb(e,t){var a=Xe();t=t===void 0?null:t;var n=a.memoizedState;if(t!==null&&Of(t,n[1]))return n[0];if(n=e(),Er){qn(!0);try{e()}finally{qn(!1)}}return a.memoizedState=[n,t],n}function If(e,t,a){return a===void 0||(er&1073741824)!==0?e.memoizedState=t:(e.memoizedState=a,e=s0(),se.lanes|=e,ar|=e,a)}function vb(e,t,a,n){return aa(a,t)?a:Ds.current!==null?(e=If(e,a,n),aa(e,t)||(dt=!0),e):(er&42)===0?(dt=!0,e.memoizedState=a):(e=s0(),se.lanes|=e,ar|=e,t)}function gb(e,t,a,n,r){var s=ge.p;ge.p=s!==0&&8>s?s:8;var i=ne.T,o={};ne.T=o,Hf(e,!1,t,a);try{var u=r(),c=ne.S;if(c!==null&&c(o,u),u!==null&&typeof u=="object"&&typeof u.then=="function"){var d=VC(u,n);no(e,t,d,ea(e))}else no(e,t,n,ea(e))}catch(f){no(e,t,{then:function(){},status:"rejected",reason:f},ea())}finally{ge.p=s,ne.T=i}}function ZC(){}function Km(e,t,a,n){if(e.tag!==5)throw Error(j(476));var r=yb(e).queue;gb(e,r,t,xr,a===null?ZC:function(){return bb(e),a(n)})}function yb(e){var t=e.memoizedState;if(t!==null)return t;t={memoizedState:xr,baseState:xr,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:vn,lastRenderedState:xr},next:null};var a={};return t.next={memoizedState:a,baseState:a,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:vn,lastRenderedState:a},next:null},e.memoizedState=t,e=e.alternate,e!==null&&(e.memoizedState=t),t}function bb(e){var t=yb(e).next.queue;no(e,t,{},ea())}function Kf(){return _t(xo)}function xb(){return Xe().memoizedState}function $b(){return Xe().memoizedState}function WC(e){for(var t=e.return;t!==null;){switch(t.tag){case 24:case 3:var a=ea();e=Vn(a);var n=Gn(t,e,a);n!==null&&(ta(n,t,a),eo(n,t,a)),t={cache:Tf()},e.payload=t;return}t=t.return}}function eE(e,t,a){var n=ea();a={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null},Zu(e)?Sb(t,a):(a=kf(e,t,a,n),a!==null&&(ta(a,e,n),Nb(a,t,n)))}function wb(e,t,a){var n=ea();no(e,t,a,n)}function no(e,t,a,n){var r={lane:n,revertLane:0,action:a,hasEagerState:!1,eagerState:null,next:null};if(Zu(e))Sb(t,r);else{var s=e.alternate;if(e.lanes===0&&(s===null||s.lanes===0)&&(s=t.lastRenderedReducer,s!==null))try{var i=t.lastRenderedState,o=s(i,a);if(r.hasEagerState=!0,r.eagerState=o,aa(o,i))return Gu(e,t,r,0),ke===null&&Vu(),!1}catch{}finally{}if(a=kf(e,t,r,n),a!==null)return ta(a,e,n),Nb(a,t,n),!0}return!1}function Hf(e,t,a,n){if(n={lane:2,revertLane:Wf(),action:n,hasEagerState:!1,eagerState:null,next:null},Zu(e)){if(t)throw Error(j(479))}else t=kf(e,a,n,2),t!==null&&ta(t,e,2)}function Zu(e){var t=e.alternate;return e===se||t!==null&&t===se}function Sb(e,t){Ns=Nu=!0;var a=e.pending;a===null?t.next=t:(t.next=a.next,a.next=t),e.pending=t}function Nb(e,t,a){if((a&4194048)!==0){var n=t.lanes;n&=e.pendingLanes,a|=n,t.lanes=a,py(e,a)}}var ku={readContext:_t,use:Ju,useCallback:Qe,useContext:Qe,useEffect:Qe,useImperativeHandle:Qe,useLayoutEffect:Qe,useInsertionEffect:Qe,useMemo:Qe,useReducer:Qe,useRef:Qe,useState:Qe,useDebugValue:Qe,useDeferredValue:Qe,useTransition:Qe,useSyncExternalStore:Qe,useId:Qe,useHostTransitionStatus:Qe,useFormState:Qe,useActionState:Qe,useOptimistic:Qe,useMemoCache:Qe,useCacheRefresh:Qe},_b={readContext:_t,use:Ju,useCallback:function(e,t){return Ft().memoizedState=[e,t===void 0?null:t],e},useContext:_t,useEffect:gg,useImperativeHandle:function(e,t,a){a=a!=null?a.concat([e]):null,ou(4194308,4,mb.bind(null,t,e),a)},useLayoutEffect:function(e,t){return ou(4194308,4,e,t)},useInsertionEffect:function(e,t){ou(4,2,e,t)},useMemo:function(e,t){var a=Ft();t=t===void 0?null:t;var n=e();if(Er){qn(!0);try{e()}finally{qn(!1)}}return a.memoizedState=[n,t],n},useReducer:function(e,t,a){var n=Ft();if(a!==void 0){var r=a(t);if(Er){qn(!0);try{a(t)}finally{qn(!1)}}}else r=t;return n.memoizedState=n.baseState=r,e={pending:null,lanes:0,dispatch:null,lastRenderedReducer:e,lastRenderedState:r},n.queue=e,e=e.dispatch=eE.bind(null,se,e),[n.memoizedState,e]},useRef:function(e){var t=Ft();return e={current:e},t.memoizedState=e},useState:function(e){e=qm(e);var t=e.queue,a=wb.bind(null,se,t);return t.dispatch=a,[e.memoizedState,a]},useDebugValue:qf,useDeferredValue:function(e,t){var a=Ft();return If(a,e,t)},useTransition:function(){var e=qm(!1);return e=gb.bind(null,se,e.queue,!0,!1),Ft().memoizedState=e,[!1,e]},useSyncExternalStore:function(e,t,a){var n=se,r=Ft();if(ve){if(a===void 0)throw Error(j(407));a=a()}else{if(a=t(),ke===null)throw Error(j(349));(de&124)!==0||Zy(n,t,a)}r.memoizedState=a;var s={value:a,getSnapshot:t};return r.queue=s,gg(eb.bind(null,n,s,e),[e]),n.flags|=2048,Ms(9,Xu(),Wy.bind(null,n,s,a,t),null),a},useId:function(){var e=Ft(),t=ke.identifierPrefix;if(ve){var a=cn,n=un;a=(n&~(1<<32-Wt(n)-1)).toString(32)+a,t="\xAB"+t+"R"+a,a=_u++,0<a&&(t+="H"+a.toString(32)),t+="\xBB"}else a=GC++,t="\xAB"+t+"r"+a.toString(32)+"\xBB";return e.memoizedState=t},useHostTransitionStatus:Kf,useFormState:pg,useActionState:pg,useOptimistic:function(e){var t=Ft();t.memoizedState=t.baseState=e;var a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return t.queue=a,t=Hf.bind(null,se,!0,a),a.dispatch=t,[e,t]},useMemoCache:Bf,useCacheRefresh:function(){return Ft().memoizedState=WC.bind(null,se)}},kb={readContext:_t,use:Ju,useCallback:pb,useContext:_t,useEffect:ub,useImperativeHandle:fb,useInsertionEffect:cb,useLayoutEffect:db,useMemo:hb,useReducer:iu,useRef:lb,useState:function(){return iu(vn)},useDebugValue:qf,useDeferredValue:function(e,t){var a=Xe();return vb(a,Ne.memoizedState,e,t)},useTransition:function(){var e=iu(vn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Oo(e),t]},useSyncExternalStore:Xy,useId:xb,useHostTransitionStatus:Kf,useFormState:hg,useActionState:hg,useOptimistic:function(e,t){var a=Xe();return nb(a,Ne,e,t)},useMemoCache:Bf,useCacheRefresh:$b},tE={readContext:_t,use:Ju,useCallback:pb,useContext:_t,useEffect:ub,useImperativeHandle:fb,useInsertionEffect:cb,useLayoutEffect:db,useMemo:hb,useReducer:em,useRef:lb,useState:function(){return em(vn)},useDebugValue:qf,useDeferredValue:function(e,t){var a=Xe();return Ne===null?If(a,e,t):vb(a,Ne.memoizedState,e,t)},useTransition:function(){var e=em(vn)[0],t=Xe().memoizedState;return[typeof e=="boolean"?e:Oo(e),t]},useSyncExternalStore:Xy,useId:xb,useHostTransitionStatus:Kf,useFormState:vg,useActionState:vg,useOptimistic:function(e,t){var a=Xe();return Ne!==null?nb(a,Ne,e,t):(a.baseState=e,[e,a.queue.dispatch])},useMemoCache:Bf,useCacheRefresh:$b},ks=null,vo=0;function Ql(e){var t=vo;return vo+=1,ks===null&&(ks=[]),Qy(ks,e,t)}function Bi(e,t){t=t.props.ref,e.ref=t!==void 0?t:null}function Vl(e,t){throw t.$$typeof===AR?Error(j(525)):(e=Object.prototype.toString.call(t),Error(j(31,e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)))}function yg(e){var t=e._init;return t(e._payload)}function Rb(e){function t(g,v){if(e){var x=g.deletions;x===null?(g.deletions=[v],g.flags|=16):x.push(v)}}function a(g,v){if(!e)return null;for(;v!==null;)t(g,v),v=v.sibling;return null}function n(g){for(var v=new Map;g!==null;)g.key!==null?v.set(g.key,g):v.set(g.index,g),g=g.sibling;return v}function r(g,v){return g=fn(g,v),g.index=0,g.sibling=null,g}function s(g,v,x){return g.index=x,e?(x=g.alternate,x!==null?(x=x.index,x<v?(g.flags|=67108866,v):x):(g.flags|=67108866,v)):(g.flags|=1048576,v)}function i(g){return e&&g.alternate===null&&(g.flags|=67108866),g}function o(g,v,x,$){return v===null||v.tag!==6?(v=Xd(x,g.mode,$),v.return=g,v):(v=r(v,x),v.return=g,v)}function u(g,v,x,$){var S=x.type;return S===ls?d(g,v,x.props.children,$,x.key):v!==null&&(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Pn&&yg(S)===v.type)?(v=r(v,x.props),Bi(v,x),v.return=g,v):(v=ru(x.type,x.key,x.props,null,g.mode,$),Bi(v,x),v.return=g,v)}function c(g,v,x,$){return v===null||v.tag!==4||v.stateNode.containerInfo!==x.containerInfo||v.stateNode.implementation!==x.implementation?(v=Zd(x,g.mode,$),v.return=g,v):(v=r(v,x.children||[]),v.return=g,v)}function d(g,v,x,$,S){return v===null||v.tag!==7?(v=$r(x,g.mode,$,S),v.return=g,v):(v=r(v,x),v.return=g,v)}function f(g,v,x){if(typeof v=="string"&&v!==""||typeof v=="number"||typeof v=="bigint")return v=Xd(""+v,g.mode,x),v.return=g,v;if(typeof v=="object"&&v!==null){switch(v.$$typeof){case Ul:return x=ru(v.type,v.key,v.props,null,g.mode,x),Bi(x,v),x.return=g,x;case Hi:return v=Zd(v,g.mode,x),v.return=g,v;case Pn:var $=v._init;return v=$(v._payload),f(g,v,x)}if(Qi(v)||Ui(v))return v=$r(v,g.mode,x,null),v.return=g,v;if(typeof v.then=="function")return f(g,Ql(v),x);if(v.$$typeof===ln)return f(g,Kl(g,v),x);Vl(g,v)}return null}function m(g,v,x,$){var S=v!==null?v.key:null;if(typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint")return S!==null?null:o(g,v,""+x,$);if(typeof x=="object"&&x!==null){switch(x.$$typeof){case Ul:return x.key===S?u(g,v,x,$):null;case Hi:return x.key===S?c(g,v,x,$):null;case Pn:return S=x._init,x=S(x._payload),m(g,v,x,$)}if(Qi(x)||Ui(x))return S!==null?null:d(g,v,x,$,null);if(typeof x.then=="function")return m(g,v,Ql(x),$);if(x.$$typeof===ln)return m(g,v,Kl(g,x),$);Vl(g,x)}return null}function h(g,v,x,$,S){if(typeof $=="string"&&$!==""||typeof $=="number"||typeof $=="bigint")return g=g.get(x)||null,o(v,g,""+$,S);if(typeof $=="object"&&$!==null){switch($.$$typeof){case Ul:return g=g.get($.key===null?x:$.key)||null,u(v,g,$,S);case Hi:return g=g.get($.key===null?x:$.key)||null,c(v,g,$,S);case Pn:var C=$._init;return $=C($._payload),h(g,v,x,$,S)}if(Qi($)||Ui($))return g=g.get(x)||null,d(v,g,$,S,null);if(typeof $.then=="function")return h(g,v,x,Ql($),S);if($.$$typeof===ln)return h(g,v,x,Kl(v,$),S);Vl(v,$)}return null}function b(g,v,x,$){for(var S=null,C=null,_=v,T=v=0,M=null;_!==null&&T<x.length;T++){_.index>T?(M=_,_=null):M=_.sibling;var O=m(g,_,x[T],$);if(O===null){_===null&&(_=M);break}e&&_&&O.alternate===null&&t(g,_),v=s(O,v,T),C===null?S=O:C.sibling=O,C=O,_=M}if(T===x.length)return a(g,_),ve&&yr(g,T),S;if(_===null){for(;T<x.length;T++)_=f(g,x[T],$),_!==null&&(v=s(_,v,T),C===null?S=_:C.sibling=_,C=_);return ve&&yr(g,T),S}for(_=n(_);T<x.length;T++)M=h(_,g,T,x[T],$),M!==null&&(e&&M.alternate!==null&&_.delete(M.key===null?T:M.key),v=s(M,v,T),C===null?S=M:C.sibling=M,C=M);return e&&_.forEach(function(U){return t(g,U)}),ve&&yr(g,T),S}function y(g,v,x,$){if(x==null)throw Error(j(151));for(var S=null,C=null,_=v,T=v=0,M=null,O=x.next();_!==null&&!O.done;T++,O=x.next()){_.index>T?(M=_,_=null):M=_.sibling;var U=m(g,_,O.value,$);if(U===null){_===null&&(_=M);break}e&&_&&U.alternate===null&&t(g,_),v=s(U,v,T),C===null?S=U:C.sibling=U,C=U,_=M}if(O.done)return a(g,_),ve&&yr(g,T),S;if(_===null){for(;!O.done;T++,O=x.next())O=f(g,O.value,$),O!==null&&(v=s(O,v,T),C===null?S=O:C.sibling=O,C=O);return ve&&yr(g,T),S}for(_=n(_);!O.done;T++,O=x.next())O=h(_,g,T,O.value,$),O!==null&&(e&&O.alternate!==null&&_.delete(O.key===null?T:O.key),v=s(O,v,T),C===null?S=O:C.sibling=O,C=O);return e&&_.forEach(function(k){return t(g,k)}),ve&&yr(g,T),S}function w(g,v,x,$){if(typeof x=="object"&&x!==null&&x.type===ls&&x.key===null&&(x=x.props.children),typeof x=="object"&&x!==null){switch(x.$$typeof){case Ul:e:{for(var S=x.key;v!==null;){if(v.key===S){if(S=x.type,S===ls){if(v.tag===7){a(g,v.sibling),$=r(v,x.props.children),$.return=g,g=$;break e}}else if(v.elementType===S||typeof S=="object"&&S!==null&&S.$$typeof===Pn&&yg(S)===v.type){a(g,v.sibling),$=r(v,x.props),Bi($,x),$.return=g,g=$;break e}a(g,v);break}else t(g,v);v=v.sibling}x.type===ls?($=$r(x.props.children,g.mode,$,x.key),$.return=g,g=$):($=ru(x.type,x.key,x.props,null,g.mode,$),Bi($,x),$.return=g,g=$)}return i(g);case Hi:e:{for(S=x.key;v!==null;){if(v.key===S)if(v.tag===4&&v.stateNode.containerInfo===x.containerInfo&&v.stateNode.implementation===x.implementation){a(g,v.sibling),$=r(v,x.children||[]),$.return=g,g=$;break e}else{a(g,v);break}else t(g,v);v=v.sibling}$=Zd(x,g.mode,$),$.return=g,g=$}return i(g);case Pn:return S=x._init,x=S(x._payload),w(g,v,x,$)}if(Qi(x))return b(g,v,x,$);if(Ui(x)){if(S=Ui(x),typeof S!="function")throw Error(j(150));return x=S.call(x),y(g,v,x,$)}if(typeof x.then=="function")return w(g,v,Ql(x),$);if(x.$$typeof===ln)return w(g,v,Kl(g,x),$);Vl(g,x)}return typeof x=="string"&&x!==""||typeof x=="number"||typeof x=="bigint"?(x=""+x,v!==null&&v.tag===6?(a(g,v.sibling),$=r(v,x),$.return=g,g=$):(a(g,v),$=Xd(x,g.mode,$),$.return=g,g=$),i(g)):a(g,v)}return function(g,v,x,$){try{vo=0;var S=w(g,v,x,$);return ks=null,S}catch(_){if(_===Mo||_===Yu)throw _;var C=Xt(29,_,null,g.mode);return C.lanes=$,C.return=g,C}finally{}}}var Os=Rb(!0),Cb=Rb(!1),xa=Ia(null),qa=null;function Fn(e){var t=e.alternate;Le(rt,rt.current&1),Le(xa,e),qa===null&&(t===null||Ds.current!==null||t.memoizedState!==null)&&(qa=e)}function Eb(e){if(e.tag===22){if(Le(rt,rt.current),Le(xa,e),qa===null){var t=e.alternate;t!==null&&t.memoizedState!==null&&(qa=e)}}else Bn(e)}function Bn(){Le(rt,rt.current),Le(xa,xa.current)}function mn(e){mt(xa),qa===e&&(qa=null),mt(rt)}var rt=Ia(0);function Ru(e){for(var t=e;t!==null;){if(t.tag===13){var a=t.memoizedState;if(a!==null&&(a=a.dehydrated,a===null||a.data==="$?"||lf(a)))return t}else if(t.tag===19&&t.memoizedProps.revealOrder!==void 0){if((t.flags&128)!==0)return t}else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return null;t=t.return}t.sibling.return=t.return,t=t.sibling}return null}function tm(e,t,a,n){t=e.memoizedState,a=a(n,t),a=a==null?t:Ae({},t,a),e.memoizedState=a,e.lanes===0&&(e.updateQueue.baseState=a)}var Hm={enqueueSetState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Vn(n);r.payload=t,a!=null&&(r.callback=a),t=Gn(e,r,n),t!==null&&(ta(t,e,n),eo(t,e,n))},enqueueReplaceState:function(e,t,a){e=e._reactInternals;var n=ea(),r=Vn(n);r.tag=1,r.payload=t,a!=null&&(r.callback=a),t=Gn(e,r,n),t!==null&&(ta(t,e,n),eo(t,e,n))},enqueueForceUpdate:function(e,t){e=e._reactInternals;var a=ea(),n=Vn(a);n.tag=2,t!=null&&(n.callback=t),t=Gn(e,n,a),t!==null&&(ta(t,e,a),eo(t,e,a))}};function bg(e,t,a,n,r,s,i){return e=e.stateNode,typeof e.shouldComponentUpdate=="function"?e.shouldComponentUpdate(n,s,i):t.prototype&&t.prototype.isPureReactComponent?!fo(a,n)||!fo(r,s):!0}function xg(e,t,a,n){e=t.state,typeof t.componentWillReceiveProps=="function"&&t.componentWillReceiveProps(a,n),typeof t.UNSAFE_componentWillReceiveProps=="function"&&t.UNSAFE_componentWillReceiveProps(a,n),t.state!==e&&Hm.enqueueReplaceState(t,t.state,null)}function Tr(e,t){var a=t;if("ref"in t){a={};for(var n in t)n!=="ref"&&(a[n]=t[n])}if(e=e.defaultProps){a===t&&(a=Ae({},a));for(var r in e)a[r]===void 0&&(a[r]=e[r])}return a}var Cu=typeof reportError=="function"?reportError:function(e){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var t=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof e=="object"&&e!==null&&typeof e.message=="string"?String(e.message):String(e),error:e});if(!window.dispatchEvent(t))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",e);return}console.error(e)};function Tb(e){Cu(e)}function Ab(e){console.error(e)}function Db(e){Cu(e)}function Eu(e,t){try{var a=e.onUncaughtError;a(t.value,{componentStack:t.stack})}catch(n){setTimeout(function(){throw n})}}function $g(e,t,a){try{var n=e.onCaughtError;n(a.value,{componentStack:a.stack,errorBoundary:t.tag===1?t.stateNode:null})}catch(r){setTimeout(function(){throw r})}}function Qm(e,t,a){return a=Vn(a),a.tag=3,a.payload={element:null},a.callback=function(){Eu(e,t)},a}function Mb(e){return e=Vn(e),e.tag=3,e}function Ob(e,t,a,n){var r=a.type.getDerivedStateFromError;if(typeof r=="function"){var s=n.value;e.payload=function(){return r(s)},e.callback=function(){$g(t,a,n)}}var i=a.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(e.callback=function(){$g(t,a,n),typeof r!="function"&&(Yn===null?Yn=new Set([this]):Yn.add(this));var o=n.stack;this.componentDidCatch(n.value,{componentStack:o!==null?o:""})})}function aE(e,t,a,n,r){if(a.flags|=32768,n!==null&&typeof n=="object"&&typeof n.then=="function"){if(t=a.alternate,t!==null&&Ao(t,a,r,!0),a=xa.current,a!==null){switch(a.tag){case 13:return qa===null?ef():a.alternate===null&&Ie===0&&(Ie=3),a.flags&=-257,a.flags|=65536,a.lanes=r,n===jm?a.flags|=16384:(t=a.updateQueue,t===null?a.updateQueue=new Set([n]):t.add(n),mm(e,n,r)),!1;case 22:return a.flags|=65536,n===jm?a.flags|=16384:(t=a.updateQueue,t===null?(t={transitions:null,markerInstances:null,retryQueue:new Set([n])},a.updateQueue=t):(a=t.retryQueue,a===null?t.retryQueue=new Set([n]):a.add(n)),mm(e,n,r)),!1}throw Error(j(435,a.tag))}return mm(e,n,r),ef(),!1}if(ve)return t=xa.current,t!==null?((t.flags&65536)===0&&(t.flags|=256),t.flags|=65536,t.lanes=r,n!==Mm&&(e=Error(j(422),{cause:n}),po(ya(e,a)))):(n!==Mm&&(t=Error(j(423),{cause:n}),po(ya(t,a))),e=e.current.alternate,e.flags|=65536,r&=-r,e.lanes|=r,n=ya(n,a),r=Qm(e.stateNode,n,r),Wd(e,r),Ie!==4&&(Ie=2)),!1;var s=Error(j(520),{cause:n});if(s=ya(s,a),io===null?io=[s]:io.push(s),Ie!==4&&(Ie=2),t===null)return!0;n=ya(n,a),a=t;do{switch(a.tag){case 3:return a.flags|=65536,e=r&-r,a.lanes|=e,e=Qm(a.stateNode,n,e),Wd(a,e),!1;case 1:if(t=a.type,s=a.stateNode,(a.flags&128)===0&&(typeof t.getDerivedStateFromError=="function"||s!==null&&typeof s.componentDidCatch=="function"&&(Yn===null||!Yn.has(s))))return a.flags|=65536,r&=-r,a.lanes|=r,r=Mb(r),Ob(r,e,a,n),Wd(a,r),!1}a=a.return}while(a!==null);return!1}var Lb=Error(j(461)),dt=!1;function vt(e,t,a,n){t.child=e===null?Cb(t,null,a,n):Os(t,e.child,a,n)}function wg(e,t,a,n,r){a=a.render;var s=t.ref;if("ref"in n){var i={};for(var o in n)o!=="ref"&&(i[o]=n[o])}else i=n;return Cr(t),n=Lf(e,t,a,i,s,r),o=Pf(),e!==null&&!dt?(Uf(e,t,r),gn(e,t,r)):(ve&&o&&Cf(t),t.flags|=1,vt(e,t,n,r),t.child)}function Sg(e,t,a,n,r){if(e===null){var s=a.type;return typeof s=="function"&&!Rf(s)&&s.defaultProps===void 0&&a.compare===null?(t.tag=15,t.type=s,Pb(e,t,s,n,r)):(e=ru(a.type,null,n,t,t.mode,r),e.ref=t.ref,e.return=t,t.child=e)}if(s=e.child,!Qf(e,r)){var i=s.memoizedProps;if(a=a.compare,a=a!==null?a:fo,a(i,n)&&e.ref===t.ref)return gn(e,t,r)}return t.flags|=1,e=fn(s,n),e.ref=t.ref,e.return=t,t.child=e}function Pb(e,t,a,n,r){if(e!==null){var s=e.memoizedProps;if(fo(s,n)&&e.ref===t.ref)if(dt=!1,t.pendingProps=n=s,Qf(e,r))(e.flags&131072)!==0&&(dt=!0);else return t.lanes=e.lanes,gn(e,t,r)}return Vm(e,t,a,n,r)}function Ub(e,t,a){var n=t.pendingProps,r=n.children,s=e!==null?e.memoizedState:null;if(n.mode==="hidden"){if((t.flags&128)!==0){if(n=s!==null?s.baseLanes|a:a,e!==null){for(r=t.child=e.child,s=0;r!==null;)s=s|r.lanes|r.childLanes,r=r.sibling;t.childLanes=s&~n}else t.childLanes=0,t.child=null;return Ng(e,t,n,a)}if((a&536870912)!==0)t.memoizedState={baseLanes:0,cachePool:null},e!==null&&su(t,s!==null?s.cachePool:null),s!==null?dg(t,s):zm(),Eb(t);else return t.lanes=t.childLanes=536870912,Ng(e,t,s!==null?s.baseLanes|a:a,a)}else s!==null?(su(t,s.cachePool),dg(t,s),Bn(t),t.memoizedState=null):(e!==null&&su(t,null),zm(),Bn(t));return vt(e,t,r,a),t.child}function Ng(e,t,a,n){var r=Af();return r=r===null?null:{parent:nt._currentValue,pool:r},t.memoizedState={baseLanes:a,cachePool:r},e!==null&&su(t,null),zm(),Eb(t),e!==null&&Ao(e,t,n,!0),null}function lu(e,t){var a=t.ref;if(a===null)e!==null&&e.ref!==null&&(t.flags|=4194816);else{if(typeof a!="function"&&typeof a!="object")throw Error(j(284));(e===null||e.ref!==a)&&(t.flags|=4194816)}}function Vm(e,t,a,n,r){return Cr(t),a=Lf(e,t,a,n,void 0,r),n=Pf(),e!==null&&!dt?(Uf(e,t,r),gn(e,t,r)):(ve&&n&&Cf(t),t.flags|=1,vt(e,t,a,r),t.child)}function _g(e,t,a,n,r,s){return Cr(t),t.updateQueue=null,a=Jy(t,n,a,r),Yy(e),n=Pf(),e!==null&&!dt?(Uf(e,t,s),gn(e,t,s)):(ve&&n&&Cf(t),t.flags|=1,vt(e,t,a,s),t.child)}function kg(e,t,a,n,r){if(Cr(t),t.stateNode===null){var s=vs,i=a.contextType;typeof i=="object"&&i!==null&&(s=_t(i)),s=new a(n,s),t.memoizedState=s.state!==null&&s.state!==void 0?s.state:null,s.updater=Hm,t.stateNode=s,s._reactInternals=t,s=t.stateNode,s.props=n,s.state=t.memoizedState,s.refs={},Df(t),i=a.contextType,s.context=typeof i=="object"&&i!==null?_t(i):vs,s.state=t.memoizedState,i=a.getDerivedStateFromProps,typeof i=="function"&&(tm(t,a,i,n),s.state=t.memoizedState),typeof a.getDerivedStateFromProps=="function"||typeof s.getSnapshotBeforeUpdate=="function"||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(i=s.state,typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount(),i!==s.state&&Hm.enqueueReplaceState(s,s.state,null),ao(t,n,s,r),to(),s.state=t.memoizedState),typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!0}else if(e===null){s=t.stateNode;var o=t.memoizedProps,u=Tr(a,o);s.props=u;var c=s.context,d=a.contextType;i=vs,typeof d=="object"&&d!==null&&(i=_t(d));var f=a.getDerivedStateFromProps;d=typeof f=="function"||typeof s.getSnapshotBeforeUpdate=="function",o=t.pendingProps!==o,d||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(o||c!==i)&&xg(t,s,n,i),Un=!1;var m=t.memoizedState;s.state=m,ao(t,n,s,r),to(),c=t.memoizedState,o||m!==c||Un?(typeof f=="function"&&(tm(t,a,f,n),c=t.memoizedState),(u=Un||bg(t,a,u,n,m,c,i))?(d||typeof s.UNSAFE_componentWillMount!="function"&&typeof s.componentWillMount!="function"||(typeof s.componentWillMount=="function"&&s.componentWillMount(),typeof s.UNSAFE_componentWillMount=="function"&&s.UNSAFE_componentWillMount()),typeof s.componentDidMount=="function"&&(t.flags|=4194308)):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),t.memoizedProps=n,t.memoizedState=c),s.props=n,s.state=c,s.context=i,n=u):(typeof s.componentDidMount=="function"&&(t.flags|=4194308),n=!1)}else{s=t.stateNode,Fm(e,t),i=t.memoizedProps,d=Tr(a,i),s.props=d,f=t.pendingProps,m=s.context,c=a.contextType,u=vs,typeof c=="object"&&c!==null&&(u=_t(c)),o=a.getDerivedStateFromProps,(c=typeof o=="function"||typeof s.getSnapshotBeforeUpdate=="function")||typeof s.UNSAFE_componentWillReceiveProps!="function"&&typeof s.componentWillReceiveProps!="function"||(i!==f||m!==u)&&xg(t,s,n,u),Un=!1,m=t.memoizedState,s.state=m,ao(t,n,s,r),to();var h=t.memoizedState;i!==f||m!==h||Un||e!==null&&e.dependencies!==null&&wu(e.dependencies)?(typeof o=="function"&&(tm(t,a,o,n),h=t.memoizedState),(d=Un||bg(t,a,d,n,m,h,u)||e!==null&&e.dependencies!==null&&wu(e.dependencies))?(c||typeof s.UNSAFE_componentWillUpdate!="function"&&typeof s.componentWillUpdate!="function"||(typeof s.componentWillUpdate=="function"&&s.componentWillUpdate(n,h,u),typeof s.UNSAFE_componentWillUpdate=="function"&&s.UNSAFE_componentWillUpdate(n,h,u)),typeof s.componentDidUpdate=="function"&&(t.flags|=4),typeof s.getSnapshotBeforeUpdate=="function"&&(t.flags|=1024)):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),t.memoizedProps=n,t.memoizedState=h),s.props=n,s.state=h,s.context=u,n=d):(typeof s.componentDidUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=4),typeof s.getSnapshotBeforeUpdate!="function"||i===e.memoizedProps&&m===e.memoizedState||(t.flags|=1024),n=!1)}return s=n,lu(e,t),n=(t.flags&128)!==0,s||n?(s=t.stateNode,a=n&&typeof a.getDerivedStateFromError!="function"?null:s.render(),t.flags|=1,e!==null&&n?(t.child=Os(t,e.child,null,r),t.child=Os(t,null,a,r)):vt(e,t,a,r),t.memoizedState=s.state,e=t.child):e=gn(e,t,r),e}function Rg(e,t,a,n){return To(),t.flags|=256,vt(e,t,a,n),t.child}var am={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function nm(e){return{baseLanes:e,cachePool:Ky()}}function rm(e,t,a){return e=e!==null?e.childLanes&~a:0,t&&(e|=ba),e}function jb(e,t,a){var n=t.pendingProps,r=!1,s=(t.flags&128)!==0,i;if((i=s)||(i=e!==null&&e.memoizedState===null?!1:(rt.current&2)!==0),i&&(r=!0,t.flags&=-129),i=(t.flags&32)!==0,t.flags&=-33,e===null){if(ve){if(r?Fn(t):Bn(t),ve){var o=qe,u;if(u=o){e:{for(u=o,o=ja;u.nodeType!==8;){if(!o){o=null;break e}if(u=ka(u.nextSibling),u===null){o=null;break e}}o=u}o!==null?(t.memoizedState={dehydrated:o,treeContext:wr!==null?{id:un,overflow:cn}:null,retryLane:536870912,hydrationErrors:null},u=Xt(18,null,null,0),u.stateNode=o,u.return=t,t.child=u,Tt=t,qe=null,u=!0):u=!1}u||Rr(t)}if(o=t.memoizedState,o!==null&&(o=o.dehydrated,o!==null))return lf(o)?t.lanes=32:t.lanes=536870912,null;mn(t)}return o=n.children,n=n.fallback,r?(Bn(t),r=t.mode,o=Tu({mode:"hidden",children:o},r),n=$r(n,r,a,null),o.return=t,n.return=t,o.sibling=n,t.child=o,r=t.child,r.memoizedState=nm(a),r.childLanes=rm(e,i,a),t.memoizedState=am,n):(Fn(t),Gm(t,o))}if(u=e.memoizedState,u!==null&&(o=u.dehydrated,o!==null)){if(s)t.flags&256?(Fn(t),t.flags&=-257,t=sm(e,t,a)):t.memoizedState!==null?(Bn(t),t.child=e.child,t.flags|=128,t=null):(Bn(t),r=n.fallback,o=t.mode,n=Tu({mode:"visible",children:n.children},o),r=$r(r,o,a,null),r.flags|=2,n.return=t,r.return=t,n.sibling=r,t.child=n,Os(t,e.child,null,a),n=t.child,n.memoizedState=nm(a),n.childLanes=rm(e,i,a),t.memoizedState=am,t=r);else if(Fn(t),lf(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var c=i.dgst;i=c,n=Error(j(419)),n.stack="",n.digest=i,po({value:n,source:null,stack:null}),t=sm(e,t,a)}else if(dt||Ao(e,t,a,!1),i=(a&e.childLanes)!==0,dt||i){if(i=ke,i!==null&&(n=a&-a,n=(n&42)!==0?1:vf(n),n=(n&(i.suspendedLanes|a))!==0?0:n,n!==0&&n!==u.retryLane))throw u.retryLane=n,zs(e,n),ta(i,e,n),Lb;o.data==="$?"||ef(),t=sm(e,t,a)}else o.data==="$?"?(t.flags|=192,t.child=e.child,t=null):(e=u.treeContext,qe=ka(o.nextSibling),Tt=t,ve=!0,Sr=null,ja=!1,e!==null&&(ha[va++]=un,ha[va++]=cn,ha[va++]=wr,un=e.id,cn=e.overflow,wr=t),t=Gm(t,n.children),t.flags|=4096);return t}return r?(Bn(t),r=n.fallback,o=t.mode,u=e.child,c=u.sibling,n=fn(u,{mode:"hidden",children:n.children}),n.subtreeFlags=u.subtreeFlags&65011712,c!==null?r=fn(c,r):(r=$r(r,o,a,null),r.flags|=2),r.return=t,n.return=t,n.sibling=r,t.child=n,n=r,r=t.child,o=e.child.memoizedState,o===null?o=nm(a):(u=o.cachePool,u!==null?(c=nt._currentValue,u=u.parent!==c?{parent:c,pool:c}:u):u=Ky(),o={baseLanes:o.baseLanes|a,cachePool:u}),r.memoizedState=o,r.childLanes=rm(e,i,a),t.memoizedState=am,n):(Fn(t),a=e.child,e=a.sibling,a=fn(a,{mode:"visible",children:n.children}),a.return=t,a.sibling=null,e!==null&&(i=t.deletions,i===null?(t.deletions=[e],t.flags|=16):i.push(e)),t.child=a,t.memoizedState=null,a)}function Gm(e,t){return t=Tu({mode:"visible",children:t},e.mode),t.return=e,e.child=t}function Tu(e,t){return e=Xt(22,e,null,t),e.lanes=0,e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null},e}function sm(e,t,a){return Os(t,e.child,null,a),e=Gm(t,t.pendingProps.children),e.flags|=2,t.memoizedState=null,e}function Cg(e,t,a){e.lanes|=t;var n=e.alternate;n!==null&&(n.lanes|=t),Lm(e.return,t,a)}function im(e,t,a,n,r){var s=e.memoizedState;s===null?e.memoizedState={isBackwards:t,rendering:null,renderingStartTime:0,last:n,tail:a,tailMode:r}:(s.isBackwards=t,s.rendering=null,s.renderingStartTime=0,s.last=n,s.tail=a,s.tailMode=r)}function Fb(e,t,a){var n=t.pendingProps,r=n.revealOrder,s=n.tail;if(vt(e,t,n.children,a),n=rt.current,(n&2)!==0)n=n&1|2,t.flags|=128;else{if(e!==null&&(e.flags&128)!==0)e:for(e=t.child;e!==null;){if(e.tag===13)e.memoizedState!==null&&Cg(e,a,t);else if(e.tag===19)Cg(e,a,t);else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break e;for(;e.sibling===null;){if(e.return===null||e.return===t)break e;e=e.return}e.sibling.return=e.return,e=e.sibling}n&=1}switch(Le(rt,n),r){case"forwards":for(a=t.child,r=null;a!==null;)e=a.alternate,e!==null&&Ru(e)===null&&(r=a),a=a.sibling;a=r,a===null?(r=t.child,t.child=null):(r=a.sibling,a.sibling=null),im(t,!1,r,a,s);break;case"backwards":for(a=null,r=t.child,t.child=null;r!==null;){if(e=r.alternate,e!==null&&Ru(e)===null){t.child=r;break}e=r.sibling,r.sibling=a,a=r,r=e}im(t,!0,a,null,s);break;case"together":im(t,!1,null,null,void 0);break;default:t.memoizedState=null}return t.child}function gn(e,t,a){if(e!==null&&(t.dependencies=e.dependencies),ar|=t.lanes,(a&t.childLanes)===0)if(e!==null){if(Ao(e,t,a,!1),(a&t.childLanes)===0)return null}else return null;if(e!==null&&t.child!==e.child)throw Error(j(153));if(t.child!==null){for(e=t.child,a=fn(e,e.pendingProps),t.child=a,a.return=t;e.sibling!==null;)e=e.sibling,a=a.sibling=fn(e,e.pendingProps),a.return=t;a.sibling=null}return t.child}function Qf(e,t){return(e.lanes&t)!==0?!0:(e=e.dependencies,!!(e!==null&&wu(e)))}function nE(e,t,a){switch(t.tag){case 3:pu(t,t.stateNode.containerInfo),jn(t,nt,e.memoizedState.cache),To();break;case 27:case 5:Sm(t);break;case 4:pu(t,t.stateNode.containerInfo);break;case 10:jn(t,t.type,t.memoizedProps.value);break;case 13:var n=t.memoizedState;if(n!==null)return n.dehydrated!==null?(Fn(t),t.flags|=128,null):(a&t.child.childLanes)!==0?jb(e,t,a):(Fn(t),e=gn(e,t,a),e!==null?e.sibling:null);Fn(t);break;case 19:var r=(e.flags&128)!==0;if(n=(a&t.childLanes)!==0,n||(Ao(e,t,a,!1),n=(a&t.childLanes)!==0),r){if(n)return Fb(e,t,a);t.flags|=128}if(r=t.memoizedState,r!==null&&(r.rendering=null,r.tail=null,r.lastEffect=null),Le(rt,rt.current),n)break;return null;case 22:case 23:return t.lanes=0,Ub(e,t,a);case 24:jn(t,nt,e.memoizedState.cache)}return gn(e,t,a)}function Bb(e,t,a){if(e!==null)if(e.memoizedProps!==t.pendingProps)dt=!0;else{if(!Qf(e,a)&&(t.flags&128)===0)return dt=!1,nE(e,t,a);dt=(e.flags&131072)!==0}else dt=!1,ve&&(t.flags&1048576)!==0&&qy(t,$u,t.index);switch(t.lanes=0,t.tag){case 16:e:{e=t.pendingProps;var n=t.elementType,r=n._init;if(n=r(n._payload),t.type=n,typeof n=="function")Rf(n)?(e=Tr(n,e),t.tag=1,t=kg(null,t,n,e,a)):(t.tag=0,t=Vm(null,t,n,e,a));else{if(n!=null){if(r=n.$$typeof,r===ff){t.tag=11,t=wg(null,t,n,e,a);break e}else if(r===pf){t.tag=14,t=Sg(null,t,n,e,a);break e}}throw t=$m(n)||n,Error(j(306,t,""))}}return t;case 0:return Vm(e,t,t.type,t.pendingProps,a);case 1:return n=t.type,r=Tr(n,t.pendingProps),kg(e,t,n,r,a);case 3:e:{if(pu(t,t.stateNode.containerInfo),e===null)throw Error(j(387));n=t.pendingProps;var s=t.memoizedState;r=s.element,Fm(e,t),ao(t,n,null,a);var i=t.memoizedState;if(n=i.cache,jn(t,nt,n),n!==s.cache&&Pm(t,[nt],a,!0),to(),n=i.element,s.isDehydrated)if(s={element:n,isDehydrated:!1,cache:i.cache},t.updateQueue.baseState=s,t.memoizedState=s,t.flags&256){t=Rg(e,t,n,a);break e}else if(n!==r){r=ya(Error(j(424)),t),po(r),t=Rg(e,t,n,a);break e}else{switch(e=t.stateNode.containerInfo,e.nodeType){case 9:e=e.body;break;default:e=e.nodeName==="HTML"?e.ownerDocument.body:e}for(qe=ka(e.firstChild),Tt=t,ve=!0,Sr=null,ja=!0,a=Cb(t,null,n,a),t.child=a;a;)a.flags=a.flags&-3|4096,a=a.sibling}else{if(To(),n===r){t=gn(e,t,a);break e}vt(e,t,n,a)}t=t.child}return t;case 26:return lu(e,t),e===null?(a=Vg(t.type,null,t.pendingProps,null))?t.memoizedState=a:ve||(a=t.type,e=t.pendingProps,n=Uu(Qn.current).createElement(a),n[Nt]=t,n[qt]=e,yt(n,a,e),ct(n),t.stateNode=n):t.memoizedState=Vg(t.type,e.memoizedProps,t.pendingProps,e.memoizedState),null;case 27:return Sm(t),e===null&&ve&&(n=t.stateNode=k0(t.type,t.pendingProps,Qn.current),Tt=t,ja=!0,r=qe,rr(t.type)?(uf=r,qe=ka(n.firstChild)):qe=r),vt(e,t,t.pendingProps.children,a),lu(e,t),e===null&&(t.flags|=4194304),t.child;case 5:return e===null&&ve&&((r=n=qe)&&(n=EE(n,t.type,t.pendingProps,ja),n!==null?(t.stateNode=n,Tt=t,qe=ka(n.firstChild),ja=!1,r=!0):r=!1),r||Rr(t)),Sm(t),r=t.type,s=t.pendingProps,i=e!==null?e.memoizedProps:null,n=s.children,sf(r,s)?n=null:i!==null&&sf(r,i)&&(t.flags|=32),t.memoizedState!==null&&(r=Lf(e,t,YC,null,null,a),xo._currentValue=r),lu(e,t),vt(e,t,n,a),t.child;case 6:return e===null&&ve&&((e=a=qe)&&(a=TE(a,t.pendingProps,ja),a!==null?(t.stateNode=a,Tt=t,qe=null,e=!0):e=!1),e||Rr(t)),null;case 13:return jb(e,t,a);case 4:return pu(t,t.stateNode.containerInfo),n=t.pendingProps,e===null?t.child=Os(t,null,n,a):vt(e,t,n,a),t.child;case 11:return wg(e,t,t.type,t.pendingProps,a);case 7:return vt(e,t,t.pendingProps,a),t.child;case 8:return vt(e,t,t.pendingProps.children,a),t.child;case 12:return vt(e,t,t.pendingProps.children,a),t.child;case 10:return n=t.pendingProps,jn(t,t.type,n.value),vt(e,t,n.children,a),t.child;case 9:return r=t.type._context,n=t.pendingProps.children,Cr(t),r=_t(r),n=n(r),t.flags|=1,vt(e,t,n,a),t.child;case 14:return Sg(e,t,t.type,t.pendingProps,a);case 15:return Pb(e,t,t.type,t.pendingProps,a);case 19:return Fb(e,t,a);case 31:return n=t.pendingProps,a=t.mode,n={mode:n.mode,children:n.children},e===null?(a=Tu(n,a),a.ref=t.ref,t.child=a,a.return=t,t=a):(a=fn(e.child,n),a.ref=t.ref,t.child=a,a.return=t,t=a),t;case 22:return Ub(e,t,a);case 24:return Cr(t),n=_t(nt),e===null?(r=Af(),r===null&&(r=ke,s=Tf(),r.pooledCache=s,s.refCount++,s!==null&&(r.pooledCacheLanes|=a),r=s),t.memoizedState={parent:n,cache:r},Df(t),jn(t,nt,r)):((e.lanes&a)!==0&&(Fm(e,t),ao(t,null,null,a),to()),r=e.memoizedState,s=t.memoizedState,r.parent!==n?(r={parent:n,cache:n},t.memoizedState=r,t.lanes===0&&(t.memoizedState=t.updateQueue.baseState=r),jn(t,nt,n)):(n=s.cache,jn(t,nt,n),n!==r.cache&&Pm(t,[nt],a,!0))),vt(e,t,t.pendingProps.children,a),t.child;case 29:throw t.pendingProps}throw Error(j(156,t.tag))}function rn(e){e.flags|=4}function Eg(e,t){if(t.type!=="stylesheet"||(t.state.loading&4)!==0)e.flags&=-16777217;else if(e.flags|=16777216,!E0(t)){if(t=xa.current,t!==null&&((de&4194048)===de?qa!==null:(de&62914560)!==de&&(de&536870912)===0||t!==qa))throw Wi=jm,Hy;e.flags|=8192}}function Gl(e,t){t!==null&&(e.flags|=4),e.flags&16384&&(t=e.tag!==22?my():536870912,e.lanes|=t,Ls|=t)}function zi(e,t){if(!ve)switch(e.tailMode){case"hidden":t=e.tail;for(var a=null;t!==null;)t.alternate!==null&&(a=t),t=t.sibling;a===null?e.tail=null:a.sibling=null;break;case"collapsed":a=e.tail;for(var n=null;a!==null;)a.alternate!==null&&(n=a),a=a.sibling;n===null?t||e.tail===null?e.tail=null:e.tail.sibling=null:n.sibling=null}}function Fe(e){var t=e.alternate!==null&&e.alternate.child===e.child,a=0,n=0;if(t)for(var r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags&65011712,n|=r.flags&65011712,r.return=e,r=r.sibling;else for(r=e.child;r!==null;)a|=r.lanes|r.childLanes,n|=r.subtreeFlags,n|=r.flags,r.return=e,r=r.sibling;return e.subtreeFlags|=n,e.childLanes=a,t}function rE(e,t,a){var n=t.pendingProps;switch(Ef(t),t.tag){case 31:case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return Fe(t),null;case 1:return Fe(t),null;case 3:return a=t.stateNode,n=null,e!==null&&(n=e.memoizedState.cache),t.memoizedState.cache!==n&&(t.flags|=2048),pn(nt),Cs(),a.pendingContext&&(a.context=a.pendingContext,a.pendingContext=null),(e===null||e.child===null)&&(Fi(t)?rn(t):e===null||e.memoizedState.isDehydrated&&(t.flags&256)===0||(t.flags|=1024,sg())),Fe(t),null;case 26:return a=t.memoizedState,e===null?(rn(t),a!==null?(Fe(t),Eg(t,a)):(Fe(t),t.flags&=-16777217)):a?a!==e.memoizedState?(rn(t),Fe(t),Eg(t,a)):(Fe(t),t.flags&=-16777217):(e.memoizedProps!==n&&rn(t),Fe(t),t.flags&=-16777217),null;case 27:hu(t),a=Qn.current;var r=t.type;if(e!==null&&t.stateNode!=null)e.memoizedProps!==n&&rn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Fe(t),null}e=Ba.current,Fi(t)?ng(t,e):(e=k0(r,n,a),t.stateNode=e,rn(t))}return Fe(t),null;case 5:if(hu(t),a=t.type,e!==null&&t.stateNode!=null)e.memoizedProps!==n&&rn(t);else{if(!n){if(t.stateNode===null)throw Error(j(166));return Fe(t),null}if(e=Ba.current,Fi(t))ng(t,e);else{switch(r=Uu(Qn.current),e){case 1:e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case 2:e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;default:switch(a){case"svg":e=r.createElementNS("http://www.w3.org/2000/svg",a);break;case"math":e=r.createElementNS("http://www.w3.org/1998/Math/MathML",a);break;case"script":e=r.createElement("div"),e.innerHTML="<script><\/script>",e=e.removeChild(e.firstChild);break;case"select":e=typeof n.is=="string"?r.createElement("select",{is:n.is}):r.createElement("select"),n.multiple?e.multiple=!0:n.size&&(e.size=n.size);break;default:e=typeof n.is=="string"?r.createElement(a,{is:n.is}):r.createElement(a)}}e[Nt]=t,e[qt]=n;e:for(r=t.child;r!==null;){if(r.tag===5||r.tag===6)e.appendChild(r.stateNode);else if(r.tag!==4&&r.tag!==27&&r.child!==null){r.child.return=r,r=r.child;continue}if(r===t)break e;for(;r.sibling===null;){if(r.return===null||r.return===t)break e;r=r.return}r.sibling.return=r.return,r=r.sibling}t.stateNode=e;e:switch(yt(e,a,n),a){case"button":case"input":case"select":case"textarea":e=!!n.autoFocus;break e;case"img":e=!0;break e;default:e=!1}e&&rn(t)}}return Fe(t),t.flags&=-16777217,null;case 6:if(e&&t.stateNode!=null)e.memoizedProps!==n&&rn(t);else{if(typeof n!="string"&&t.stateNode===null)throw Error(j(166));if(e=Qn.current,Fi(t)){if(e=t.stateNode,a=t.memoizedProps,n=null,r=Tt,r!==null)switch(r.tag){case 27:case 5:n=r.memoizedProps}e[Nt]=t,e=!!(e.nodeValue===a||n!==null&&n.suppressHydrationWarning===!0||S0(e.nodeValue,a)),e||Rr(t)}else e=Uu(e).createTextNode(n),e[Nt]=t,t.stateNode=e}return Fe(t),null;case 13:if(n=t.memoizedState,e===null||e.memoizedState!==null&&e.memoizedState.dehydrated!==null){if(r=Fi(t),n!==null&&n.dehydrated!==null){if(e===null){if(!r)throw Error(j(318));if(r=t.memoizedState,r=r!==null?r.dehydrated:null,!r)throw Error(j(317));r[Nt]=t}else To(),(t.flags&128)===0&&(t.memoizedState=null),t.flags|=4;Fe(t),r=!1}else r=sg(),e!==null&&e.memoizedState!==null&&(e.memoizedState.hydrationErrors=r),r=!0;if(!r)return t.flags&256?(mn(t),t):(mn(t),null)}if(mn(t),(t.flags&128)!==0)return t.lanes=a,t;if(a=n!==null,e=e!==null&&e.memoizedState!==null,a){n=t.child,r=null,n.alternate!==null&&n.alternate.memoizedState!==null&&n.alternate.memoizedState.cachePool!==null&&(r=n.alternate.memoizedState.cachePool.pool);var s=null;n.memoizedState!==null&&n.memoizedState.cachePool!==null&&(s=n.memoizedState.cachePool.pool),s!==r&&(n.flags|=2048)}return a!==e&&a&&(t.child.flags|=8192),Gl(t,t.updateQueue),Fe(t),null;case 4:return Cs(),e===null&&ep(t.stateNode.containerInfo),Fe(t),null;case 10:return pn(t.type),Fe(t),null;case 19:if(mt(rt),r=t.memoizedState,r===null)return Fe(t),null;if(n=(t.flags&128)!==0,s=r.rendering,s===null)if(n)zi(r,!1);else{if(Ie!==0||e!==null&&(e.flags&128)!==0)for(e=t.child;e!==null;){if(s=Ru(e),s!==null){for(t.flags|=128,zi(r,!1),e=s.updateQueue,t.updateQueue=e,Gl(t,e),t.subtreeFlags=0,e=a,a=t.child;a!==null;)zy(a,e),a=a.sibling;return Le(rt,rt.current&1|2),t.child}e=e.sibling}r.tail!==null&&za()>Du&&(t.flags|=128,n=!0,zi(r,!1),t.lanes=4194304)}else{if(!n)if(e=Ru(s),e!==null){if(t.flags|=128,n=!0,e=e.updateQueue,t.updateQueue=e,Gl(t,e),zi(r,!0),r.tail===null&&r.tailMode==="hidden"&&!s.alternate&&!ve)return Fe(t),null}else 2*za()-r.renderingStartTime>Du&&a!==536870912&&(t.flags|=128,n=!0,zi(r,!1),t.lanes=4194304);r.isBackwards?(s.sibling=t.child,t.child=s):(e=r.last,e!==null?e.sibling=s:t.child=s,r.last=s)}return r.tail!==null?(t=r.tail,r.rendering=t,r.tail=t.sibling,r.renderingStartTime=za(),t.sibling=null,e=rt.current,Le(rt,n?e&1|2:e&1),t):(Fe(t),null);case 22:case 23:return mn(t),Mf(),n=t.memoizedState!==null,e!==null?e.memoizedState!==null!==n&&(t.flags|=8192):n&&(t.flags|=8192),n?(a&536870912)!==0&&(t.flags&128)===0&&(Fe(t),t.subtreeFlags&6&&(t.flags|=8192)):Fe(t),a=t.updateQueue,a!==null&&Gl(t,a.retryQueue),a=null,e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),n=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(n=t.memoizedState.cachePool.pool),n!==a&&(t.flags|=2048),e!==null&&mt(Nr),null;case 24:return a=null,e!==null&&(a=e.memoizedState.cache),t.memoizedState.cache!==a&&(t.flags|=2048),pn(nt),Fe(t),null;case 25:return null;case 30:return null}throw Error(j(156,t.tag))}function sE(e,t){switch(Ef(t),t.tag){case 1:return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 3:return pn(nt),Cs(),e=t.flags,(e&65536)!==0&&(e&128)===0?(t.flags=e&-65537|128,t):null;case 26:case 27:case 5:return hu(t),null;case 13:if(mn(t),e=t.memoizedState,e!==null&&e.dehydrated!==null){if(t.alternate===null)throw Error(j(340));To()}return e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 19:return mt(rt),null;case 4:return Cs(),null;case 10:return pn(t.type),null;case 22:case 23:return mn(t),Mf(),e!==null&&mt(Nr),e=t.flags,e&65536?(t.flags=e&-65537|128,t):null;case 24:return pn(nt),null;case 25:return null;default:return null}}function zb(e,t){switch(Ef(t),t.tag){case 3:pn(nt),Cs();break;case 26:case 27:case 5:hu(t);break;case 4:Cs();break;case 13:mn(t);break;case 19:mt(rt);break;case 10:pn(t.type);break;case 22:case 23:mn(t),Mf(),e!==null&&mt(Nr);break;case 24:pn(nt)}}function Po(e,t){try{var a=t.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var r=n.next;a=r;do{if((a.tag&e)===e){n=void 0;var s=a.create,i=a.inst;n=s(),i.destroy=n}a=a.next}while(a!==r)}}catch(o){_e(t,t.return,o)}}function tr(e,t,a){try{var n=t.updateQueue,r=n!==null?n.lastEffect:null;if(r!==null){var s=r.next;n=s;do{if((n.tag&e)===e){var i=n.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,r=t;var u=a,c=o;try{c()}catch(d){_e(r,u,d)}}}n=n.next}while(n!==s)}}catch(d){_e(t,t.return,d)}}function qb(e){var t=e.updateQueue;if(t!==null){var a=e.stateNode;try{Gy(t,a)}catch(n){_e(e,e.return,n)}}}function Ib(e,t,a){a.props=Tr(e.type,e.memoizedProps),a.state=e.memoizedState;try{a.componentWillUnmount()}catch(n){_e(e,t,n)}}function ro(e,t){try{var a=e.ref;if(a!==null){switch(e.tag){case 26:case 27:case 5:var n=e.stateNode;break;case 30:n=e.stateNode;break;default:n=e.stateNode}typeof a=="function"?e.refCleanup=a(n):a.current=n}}catch(r){_e(e,t,r)}}function Fa(e,t){var a=e.ref,n=e.refCleanup;if(a!==null)if(typeof n=="function")try{n()}catch(r){_e(e,t,r)}finally{e.refCleanup=null,e=e.alternate,e!=null&&(e.refCleanup=null)}else if(typeof a=="function")try{a(null)}catch(r){_e(e,t,r)}else a.current=null}function Kb(e){var t=e.type,a=e.memoizedProps,n=e.stateNode;try{e:switch(t){case"button":case"input":case"select":case"textarea":a.autoFocus&&n.focus();break e;case"img":a.src?n.src=a.src:a.srcSet&&(n.srcset=a.srcSet)}}catch(r){_e(e,e.return,r)}}function om(e,t,a){try{var n=e.stateNode;NE(n,e.type,a,t),n[qt]=t}catch(r){_e(e,e.return,r)}}function Hb(e){return e.tag===5||e.tag===3||e.tag===26||e.tag===27&&rr(e.type)||e.tag===4}function lm(e){e:for(;;){for(;e.sibling===null;){if(e.return===null||Hb(e.return))return null;e=e.return}for(e.sibling.return=e.return,e=e.sibling;e.tag!==5&&e.tag!==6&&e.tag!==18;){if(e.tag===27&&rr(e.type)||e.flags&2||e.child===null||e.tag===4)continue e;e.child.return=e,e=e.child}if(!(e.flags&2))return e.stateNode}}function Ym(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?(a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a).insertBefore(e,t):(t=a.nodeType===9?a.body:a.nodeName==="HTML"?a.ownerDocument.body:a,t.appendChild(e),a=a._reactRootContainer,a!=null||t.onclick!==null||(t.onclick=ac));else if(n!==4&&(n===27&&rr(e.type)&&(a=e.stateNode,t=null),e=e.child,e!==null))for(Ym(e,t,a),e=e.sibling;e!==null;)Ym(e,t,a),e=e.sibling}function Au(e,t,a){var n=e.tag;if(n===5||n===6)e=e.stateNode,t?a.insertBefore(e,t):a.appendChild(e);else if(n!==4&&(n===27&&rr(e.type)&&(a=e.stateNode),e=e.child,e!==null))for(Au(e,t,a),e=e.sibling;e!==null;)Au(e,t,a),e=e.sibling}function Qb(e){var t=e.stateNode,a=e.memoizedProps;try{for(var n=e.type,r=t.attributes;r.length;)t.removeAttributeNode(r[0]);yt(t,n,a),t[Nt]=e,t[qt]=a}catch(s){_e(e,e.return,s)}}var on=!1,Ve=!1,um=!1,Tg=typeof WeakSet=="function"?WeakSet:Set,ut=null;function iE(e,t){if(e=e.containerInfo,nf=zu,e=My(e),Nf(e)){if("selectionStart"in e)var a={start:e.selectionStart,end:e.selectionEnd};else e:{a=(a=e.ownerDocument)&&a.defaultView||window;var n=a.getSelection&&a.getSelection();if(n&&n.rangeCount!==0){a=n.anchorNode;var r=n.anchorOffset,s=n.focusNode;n=n.focusOffset;try{a.nodeType,s.nodeType}catch{a=null;break e}var i=0,o=-1,u=-1,c=0,d=0,f=e,m=null;t:for(;;){for(var h;f!==a||r!==0&&f.nodeType!==3||(o=i+r),f!==s||n!==0&&f.nodeType!==3||(u=i+n),f.nodeType===3&&(i+=f.nodeValue.length),(h=f.firstChild)!==null;)m=f,f=h;for(;;){if(f===e)break t;if(m===a&&++c===r&&(o=i),m===s&&++d===n&&(u=i),(h=f.nextSibling)!==null)break;f=m,m=f.parentNode}f=h}a=o===-1||u===-1?null:{start:o,end:u}}else a=null}a=a||{start:0,end:0}}else a=null;for(rf={focusedElem:e,selectionRange:a},zu=!1,ut=t;ut!==null;)if(t=ut,e=t.child,(t.subtreeFlags&1024)!==0&&e!==null)e.return=t,ut=e;else for(;ut!==null;){switch(t=ut,s=t.alternate,e=t.flags,t.tag){case 0:break;case 11:case 15:break;case 1:if((e&1024)!==0&&s!==null){e=void 0,a=t,r=s.memoizedProps,s=s.memoizedState,n=a.stateNode;try{var b=Tr(a.type,r,a.elementType===a.type);e=n.getSnapshotBeforeUpdate(b,s),n.__reactInternalSnapshotBeforeUpdate=e}catch(y){_e(a,a.return,y)}}break;case 3:if((e&1024)!==0){if(e=t.stateNode.containerInfo,a=e.nodeType,a===9)of(e);else if(a===1)switch(e.nodeName){case"HEAD":case"HTML":case"BODY":of(e);break;default:e.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if((e&1024)!==0)throw Error(j(163))}if(e=t.sibling,e!==null){e.return=t.return,ut=e;break}ut=t.return}}function Vb(e,t,a){var n=a.flags;switch(a.tag){case 0:case 11:case 15:On(e,a),n&4&&Po(5,a);break;case 1:if(On(e,a),n&4)if(e=a.stateNode,t===null)try{e.componentDidMount()}catch(i){_e(a,a.return,i)}else{var r=Tr(a.type,t.memoizedProps);t=t.memoizedState;try{e.componentDidUpdate(r,t,e.__reactInternalSnapshotBeforeUpdate)}catch(i){_e(a,a.return,i)}}n&64&&qb(a),n&512&&ro(a,a.return);break;case 3:if(On(e,a),n&64&&(e=a.updateQueue,e!==null)){if(t=null,a.child!==null)switch(a.child.tag){case 27:case 5:t=a.child.stateNode;break;case 1:t=a.child.stateNode}try{Gy(e,t)}catch(i){_e(a,a.return,i)}}break;case 27:t===null&&n&4&&Qb(a);case 26:case 5:On(e,a),t===null&&n&4&&Kb(a),n&512&&ro(a,a.return);break;case 12:On(e,a);break;case 13:On(e,a),n&4&&Jb(e,a),n&64&&(e=a.memoizedState,e!==null&&(e=e.dehydrated,e!==null&&(a=hE.bind(null,a),AE(e,a))));break;case 22:if(n=a.memoizedState!==null||on,!n){t=t!==null&&t.memoizedState!==null||Ve,r=on;var s=Ve;on=n,(Ve=t)&&!s?Ln(e,a,(a.subtreeFlags&8772)!==0):On(e,a),on=r,Ve=s}break;case 30:break;default:On(e,a)}}function Gb(e){var t=e.alternate;t!==null&&(e.alternate=null,Gb(t)),e.child=null,e.deletions=null,e.sibling=null,e.tag===5&&(t=e.stateNode,t!==null&&yf(t)),e.stateNode=null,e.return=null,e.dependencies=null,e.memoizedProps=null,e.memoizedState=null,e.pendingProps=null,e.stateNode=null,e.updateQueue=null}var Oe=null,Bt=!1;function sn(e,t,a){for(a=a.child;a!==null;)Yb(e,t,a),a=a.sibling}function Yb(e,t,a){if(Zt&&typeof Zt.onCommitFiberUnmount=="function")try{Zt.onCommitFiberUnmount(_o,a)}catch{}switch(a.tag){case 26:Ve||Fa(a,t),sn(e,t,a),a.memoizedState?a.memoizedState.count--:a.stateNode&&(a=a.stateNode,a.parentNode.removeChild(a));break;case 27:Ve||Fa(a,t);var n=Oe,r=Bt;rr(a.type)&&(Oe=a.stateNode,Bt=!1),sn(e,t,a),lo(a.stateNode),Oe=n,Bt=r;break;case 5:Ve||Fa(a,t);case 6:if(n=Oe,r=Bt,Oe=null,sn(e,t,a),Oe=n,Bt=r,Oe!==null)if(Bt)try{(Oe.nodeType===9?Oe.body:Oe.nodeName==="HTML"?Oe.ownerDocument.body:Oe).removeChild(a.stateNode)}catch(s){_e(a,t,s)}else try{Oe.removeChild(a.stateNode)}catch(s){_e(a,t,s)}break;case 18:Oe!==null&&(Bt?(e=Oe,Kg(e.nodeType===9?e.body:e.nodeName==="HTML"?e.ownerDocument.body:e,a.stateNode),So(e)):Kg(Oe,a.stateNode));break;case 4:n=Oe,r=Bt,Oe=a.stateNode.containerInfo,Bt=!0,sn(e,t,a),Oe=n,Bt=r;break;case 0:case 11:case 14:case 15:Ve||tr(2,a,t),Ve||tr(4,a,t),sn(e,t,a);break;case 1:Ve||(Fa(a,t),n=a.stateNode,typeof n.componentWillUnmount=="function"&&Ib(a,t,n)),sn(e,t,a);break;case 21:sn(e,t,a);break;case 22:Ve=(n=Ve)||a.memoizedState!==null,sn(e,t,a),Ve=n;break;default:sn(e,t,a)}}function Jb(e,t){if(t.memoizedState===null&&(e=t.alternate,e!==null&&(e=e.memoizedState,e!==null&&(e=e.dehydrated,e!==null))))try{So(e)}catch(a){_e(t,t.return,a)}}function oE(e){switch(e.tag){case 13:case 19:var t=e.stateNode;return t===null&&(t=e.stateNode=new Tg),t;case 22:return e=e.stateNode,t=e._retryCache,t===null&&(t=e._retryCache=new Tg),t;default:throw Error(j(435,e.tag))}}function cm(e,t){var a=oE(e);t.forEach(function(n){var r=vE.bind(null,e,n);a.has(n)||(a.add(n),n.then(r,r))})}function Gt(e,t){var a=t.deletions;if(a!==null)for(var n=0;n<a.length;n++){var r=a[n],s=e,i=t,o=i;e:for(;o!==null;){switch(o.tag){case 27:if(rr(o.type)){Oe=o.stateNode,Bt=!1;break e}break;case 5:Oe=o.stateNode,Bt=!1;break e;case 3:case 4:Oe=o.stateNode.containerInfo,Bt=!0;break e}o=o.return}if(Oe===null)throw Error(j(160));Yb(s,i,r),Oe=null,Bt=!1,s=r.alternate,s!==null&&(s.return=null),r.return=null}if(t.subtreeFlags&13878)for(t=t.child;t!==null;)Xb(t,e),t=t.sibling}var _a=null;function Xb(e,t){var a=e.alternate,n=e.flags;switch(e.tag){case 0:case 11:case 14:case 15:Gt(t,e),Yt(e),n&4&&(tr(3,e,e.return),Po(3,e),tr(5,e,e.return));break;case 1:Gt(t,e),Yt(e),n&512&&(Ve||a===null||Fa(a,a.return)),n&64&&on&&(e=e.updateQueue,e!==null&&(n=e.callbacks,n!==null&&(a=e.shared.hiddenCallbacks,e.shared.hiddenCallbacks=a===null?n:a.concat(n))));break;case 26:var r=_a;if(Gt(t,e),Yt(e),n&512&&(Ve||a===null||Fa(a,a.return)),n&4){var s=a!==null?a.memoizedState:null;if(n=e.memoizedState,a===null)if(n===null)if(e.stateNode===null){e:{n=e.type,a=e.memoizedProps,r=r.ownerDocument||r;t:switch(n){case"title":s=r.getElementsByTagName("title")[0],(!s||s[Co]||s[Nt]||s.namespaceURI==="http://www.w3.org/2000/svg"||s.hasAttribute("itemprop"))&&(s=r.createElement(n),r.head.insertBefore(s,r.querySelector("head > title"))),yt(s,n,a),s[Nt]=e,ct(s),n=s;break e;case"link":var i=Yg("link","href",r).get(n+(a.href||""));if(i){for(var o=0;o<i.length;o++)if(s=i[o],s.getAttribute("href")===(a.href==null||a.href===""?null:a.href)&&s.getAttribute("rel")===(a.rel==null?null:a.rel)&&s.getAttribute("title")===(a.title==null?null:a.title)&&s.getAttribute("crossorigin")===(a.crossOrigin==null?null:a.crossOrigin)){i.splice(o,1);break t}}s=r.createElement(n),yt(s,n,a),r.head.appendChild(s);break;case"meta":if(i=Yg("meta","content",r).get(n+(a.content||""))){for(o=0;o<i.length;o++)if(s=i[o],s.getAttribute("content")===(a.content==null?null:""+a.content)&&s.getAttribute("name")===(a.name==null?null:a.name)&&s.getAttribute("property")===(a.property==null?null:a.property)&&s.getAttribute("http-equiv")===(a.httpEquiv==null?null:a.httpEquiv)&&s.getAttribute("charset")===(a.charSet==null?null:a.charSet)){i.splice(o,1);break t}}s=r.createElement(n),yt(s,n,a),r.head.appendChild(s);break;default:throw Error(j(468,n))}s[Nt]=e,ct(s),n=s}e.stateNode=n}else Jg(r,e.type,e.stateNode);else e.stateNode=Gg(r,n,e.memoizedProps);else s!==n?(s===null?a.stateNode!==null&&(a=a.stateNode,a.parentNode.removeChild(a)):s.count--,n===null?Jg(r,e.type,e.stateNode):Gg(r,n,e.memoizedProps)):n===null&&e.stateNode!==null&&om(e,e.memoizedProps,a.memoizedProps)}break;case 27:Gt(t,e),Yt(e),n&512&&(Ve||a===null||Fa(a,a.return)),a!==null&&n&4&&om(e,e.memoizedProps,a.memoizedProps);break;case 5:if(Gt(t,e),Yt(e),n&512&&(Ve||a===null||Fa(a,a.return)),e.flags&32){r=e.stateNode;try{Ts(r,"")}catch(h){_e(e,e.return,h)}}n&4&&e.stateNode!=null&&(r=e.memoizedProps,om(e,r,a!==null?a.memoizedProps:r)),n&1024&&(um=!0);break;case 6:if(Gt(t,e),Yt(e),n&4){if(e.stateNode===null)throw Error(j(162));n=e.memoizedProps,a=e.stateNode;try{a.nodeValue=n}catch(h){_e(e,e.return,h)}}break;case 3:if(du=null,r=_a,_a=ju(t.containerInfo),Gt(t,e),_a=r,Yt(e),n&4&&a!==null&&a.memoizedState.isDehydrated)try{So(t.containerInfo)}catch(h){_e(e,e.return,h)}um&&(um=!1,Zb(e));break;case 4:n=_a,_a=ju(e.stateNode.containerInfo),Gt(t,e),Yt(e),_a=n;break;case 12:Gt(t,e),Yt(e);break;case 13:Gt(t,e),Yt(e),e.child.flags&8192&&e.memoizedState!==null!=(a!==null&&a.memoizedState!==null)&&(Xf=za()),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,cm(e,n)));break;case 22:r=e.memoizedState!==null;var u=a!==null&&a.memoizedState!==null,c=on,d=Ve;if(on=c||r,Ve=d||u,Gt(t,e),Ve=d,on=c,Yt(e),n&8192)e:for(t=e.stateNode,t._visibility=r?t._visibility&-2:t._visibility|1,r&&(a===null||u||on||Ve||br(e)),a=null,t=e;;){if(t.tag===5||t.tag===26){if(a===null){u=a=t;try{if(s=u.stateNode,r)i=s.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=u.stateNode;var f=u.memoizedProps.style,m=f!=null&&f.hasOwnProperty("display")?f.display:null;o.style.display=m==null||typeof m=="boolean"?"":(""+m).trim()}}catch(h){_e(u,u.return,h)}}}else if(t.tag===6){if(a===null){u=t;try{u.stateNode.nodeValue=r?"":u.memoizedProps}catch(h){_e(u,u.return,h)}}}else if((t.tag!==22&&t.tag!==23||t.memoizedState===null||t===e)&&t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break e;for(;t.sibling===null;){if(t.return===null||t.return===e)break e;a===t&&(a=null),t=t.return}a===t&&(a=null),t.sibling.return=t.return,t=t.sibling}n&4&&(n=e.updateQueue,n!==null&&(a=n.retryQueue,a!==null&&(n.retryQueue=null,cm(e,a))));break;case 19:Gt(t,e),Yt(e),n&4&&(n=e.updateQueue,n!==null&&(e.updateQueue=null,cm(e,n)));break;case 30:break;case 21:break;default:Gt(t,e),Yt(e)}}function Yt(e){var t=e.flags;if(t&2){try{for(var a,n=e.return;n!==null;){if(Hb(n)){a=n;break}n=n.return}if(a==null)throw Error(j(160));switch(a.tag){case 27:var r=a.stateNode,s=lm(e);Au(e,s,r);break;case 5:var i=a.stateNode;a.flags&32&&(Ts(i,""),a.flags&=-33);var o=lm(e);Au(e,o,i);break;case 3:case 4:var u=a.stateNode.containerInfo,c=lm(e);Ym(e,c,u);break;default:throw Error(j(161))}}catch(d){_e(e,e.return,d)}e.flags&=-3}t&4096&&(e.flags&=-4097)}function Zb(e){if(e.subtreeFlags&1024)for(e=e.child;e!==null;){var t=e;Zb(t),t.tag===5&&t.flags&1024&&t.stateNode.reset(),e=e.sibling}}function On(e,t){if(t.subtreeFlags&8772)for(t=t.child;t!==null;)Vb(e,t.alternate,t),t=t.sibling}function br(e){for(e=e.child;e!==null;){var t=e;switch(t.tag){case 0:case 11:case 14:case 15:tr(4,t,t.return),br(t);break;case 1:Fa(t,t.return);var a=t.stateNode;typeof a.componentWillUnmount=="function"&&Ib(t,t.return,a),br(t);break;case 27:lo(t.stateNode);case 26:case 5:Fa(t,t.return),br(t);break;case 22:t.memoizedState===null&&br(t);break;case 30:br(t);break;default:br(t)}e=e.sibling}}function Ln(e,t,a){for(a=a&&(t.subtreeFlags&8772)!==0,t=t.child;t!==null;){var n=t.alternate,r=e,s=t,i=s.flags;switch(s.tag){case 0:case 11:case 15:Ln(r,s,a),Po(4,s);break;case 1:if(Ln(r,s,a),n=s,r=n.stateNode,typeof r.componentDidMount=="function")try{r.componentDidMount()}catch(c){_e(n,n.return,c)}if(n=s,r=n.updateQueue,r!==null){var o=n.stateNode;try{var u=r.shared.hiddenCallbacks;if(u!==null)for(r.shared.hiddenCallbacks=null,r=0;r<u.length;r++)Vy(u[r],o)}catch(c){_e(n,n.return,c)}}a&&i&64&&qb(s),ro(s,s.return);break;case 27:Qb(s);case 26:case 5:Ln(r,s,a),a&&n===null&&i&4&&Kb(s),ro(s,s.return);break;case 12:Ln(r,s,a);break;case 13:Ln(r,s,a),a&&i&4&&Jb(r,s);break;case 22:s.memoizedState===null&&Ln(r,s,a),ro(s,s.return);break;case 30:break;default:Ln(r,s,a)}t=t.sibling}}function Vf(e,t){var a=null;e!==null&&e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),e=null,t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(e=t.memoizedState.cachePool.pool),e!==a&&(e!=null&&e.refCount++,a!=null&&Do(a))}function Gf(e,t){e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Do(e))}function Ua(e,t,a,n){if(t.subtreeFlags&10256)for(t=t.child;t!==null;)Wb(e,t,a,n),t=t.sibling}function Wb(e,t,a,n){var r=t.flags;switch(t.tag){case 0:case 11:case 15:Ua(e,t,a,n),r&2048&&Po(9,t);break;case 1:Ua(e,t,a,n);break;case 3:Ua(e,t,a,n),r&2048&&(e=null,t.alternate!==null&&(e=t.alternate.memoizedState.cache),t=t.memoizedState.cache,t!==e&&(t.refCount++,e!=null&&Do(e)));break;case 12:if(r&2048){Ua(e,t,a,n),e=t.stateNode;try{var s=t.memoizedProps,i=s.id,o=s.onPostCommit;typeof o=="function"&&o(i,t.alternate===null?"mount":"update",e.passiveEffectDuration,-0)}catch(u){_e(t,t.return,u)}}else Ua(e,t,a,n);break;case 13:Ua(e,t,a,n);break;case 23:break;case 22:s=t.stateNode,i=t.alternate,t.memoizedState!==null?s._visibility&2?Ua(e,t,a,n):so(e,t):s._visibility&2?Ua(e,t,a,n):(s._visibility|=2,is(e,t,a,n,(t.subtreeFlags&10256)!==0)),r&2048&&Vf(i,t);break;case 24:Ua(e,t,a,n),r&2048&&Gf(t.alternate,t);break;default:Ua(e,t,a,n)}}function is(e,t,a,n,r){for(r=r&&(t.subtreeFlags&10256)!==0,t=t.child;t!==null;){var s=e,i=t,o=a,u=n,c=i.flags;switch(i.tag){case 0:case 11:case 15:is(s,i,o,u,r),Po(8,i);break;case 23:break;case 22:var d=i.stateNode;i.memoizedState!==null?d._visibility&2?is(s,i,o,u,r):so(s,i):(d._visibility|=2,is(s,i,o,u,r)),r&&c&2048&&Vf(i.alternate,i);break;case 24:is(s,i,o,u,r),r&&c&2048&&Gf(i.alternate,i);break;default:is(s,i,o,u,r)}t=t.sibling}}function so(e,t){if(t.subtreeFlags&10256)for(t=t.child;t!==null;){var a=e,n=t,r=n.flags;switch(n.tag){case 22:so(a,n),r&2048&&Vf(n.alternate,n);break;case 24:so(a,n),r&2048&&Gf(n.alternate,n);break;default:so(a,n)}t=t.sibling}}var Gi=8192;function ns(e){if(e.subtreeFlags&Gi)for(e=e.child;e!==null;)e0(e),e=e.sibling}function e0(e){switch(e.tag){case 26:ns(e),e.flags&Gi&&e.memoizedState!==null&&KE(_a,e.memoizedState,e.memoizedProps);break;case 5:ns(e);break;case 3:case 4:var t=_a;_a=ju(e.stateNode.containerInfo),ns(e),_a=t;break;case 22:e.memoizedState===null&&(t=e.alternate,t!==null&&t.memoizedState!==null?(t=Gi,Gi=16777216,ns(e),Gi=t):ns(e));break;default:ns(e)}}function t0(e){var t=e.alternate;if(t!==null&&(e=t.child,e!==null)){t.child=null;do t=e.sibling,e.sibling=null,e=t;while(e!==null)}}function qi(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,n0(n,e)}t0(e)}if(e.subtreeFlags&10256)for(e=e.child;e!==null;)a0(e),e=e.sibling}function a0(e){switch(e.tag){case 0:case 11:case 15:qi(e),e.flags&2048&&tr(9,e,e.return);break;case 3:qi(e);break;case 12:qi(e);break;case 22:var t=e.stateNode;e.memoizedState!==null&&t._visibility&2&&(e.return===null||e.return.tag!==13)?(t._visibility&=-3,uu(e)):qi(e);break;default:qi(e)}}function uu(e){var t=e.deletions;if((e.flags&16)!==0){if(t!==null)for(var a=0;a<t.length;a++){var n=t[a];ut=n,n0(n,e)}t0(e)}for(e=e.child;e!==null;){switch(t=e,t.tag){case 0:case 11:case 15:tr(8,t,t.return),uu(t);break;case 22:a=t.stateNode,a._visibility&2&&(a._visibility&=-3,uu(t));break;default:uu(t)}e=e.sibling}}function n0(e,t){for(;ut!==null;){var a=ut;switch(a.tag){case 0:case 11:case 15:tr(8,a,t);break;case 23:case 22:if(a.memoizedState!==null&&a.memoizedState.cachePool!==null){var n=a.memoizedState.cachePool.pool;n!=null&&n.refCount++}break;case 24:Do(a.memoizedState.cache)}if(n=a.child,n!==null)n.return=a,ut=n;else e:for(a=e;ut!==null;){n=ut;var r=n.sibling,s=n.return;if(Gb(n),n===a){ut=null;break e}if(r!==null){r.return=s,ut=r;break e}ut=s}}}var lE={getCacheForType:function(e){var t=_t(nt),a=t.data.get(e);return a===void 0&&(a=e(),t.data.set(e,a)),a}},uE=typeof WeakMap=="function"?WeakMap:Map,we=0,ke=null,ue=null,de=0,$e=0,Jt=null,Kn=!1,qs=!1,Yf=!1,yn=0,Ie=0,ar=0,_r=0,Jf=0,ba=0,Ls=0,io=null,zt=null,Jm=!1,Xf=0,Du=1/0,Mu=null,Yn=null,gt=0,Jn=null,Ps=null,Rs=0,Xm=0,Zm=null,r0=null,oo=0,Wm=null;function ea(){if((we&2)!==0&&de!==0)return de&-de;if(ne.T!==null){var e=As;return e!==0?e:Wf()}return hy()}function s0(){ba===0&&(ba=(de&536870912)===0||ve?dy():536870912);var e=xa.current;return e!==null&&(e.flags|=32),ba}function ta(e,t,a){(e===ke&&($e===2||$e===9)||e.cancelPendingCommit!==null)&&(Us(e,0),Hn(e,de,ba,!1)),Ro(e,a),((we&2)===0||e!==ke)&&(e===ke&&((we&2)===0&&(_r|=a),Ie===4&&Hn(e,de,ba,!1)),Ka(e))}function i0(e,t,a){if((we&6)!==0)throw Error(j(327));var n=!a&&(t&124)===0&&(t&e.expiredLanes)===0||ko(e,t),r=n?mE(e,t):dm(e,t,!0),s=n;do{if(r===0){qs&&!n&&Hn(e,t,0,!1);break}else{if(a=e.current.alternate,s&&!cE(a)){r=dm(e,t,!1),s=!1;continue}if(r===2){if(s=t,e.errorRecoveryDisabledLanes&s)var i=0;else i=e.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){t=i;e:{var o=e;r=io;var u=o.current.memoizedState.isDehydrated;if(u&&(Us(o,i).flags|=256),i=dm(o,i,!1),i!==2){if(Yf&&!u){o.errorRecoveryDisabledLanes|=s,_r|=s,r=4;break e}s=zt,zt=r,s!==null&&(zt===null?zt=s:zt.push.apply(zt,s))}r=i}if(s=!1,r!==2)continue}}if(r===1){Us(e,0),Hn(e,t,0,!0);break}e:{switch(n=e,s=r,s){case 0:case 1:throw Error(j(345));case 4:if((t&4194048)!==t)break;case 6:Hn(n,t,ba,!Kn);break e;case 2:zt=null;break;case 3:case 5:break;default:throw Error(j(329))}if((t&62914560)===t&&(r=Xf+300-za(),10<r)){if(Hn(n,t,ba,!Kn),Iu(n,0,!0)!==0)break e;n.timeoutHandle=_0(Ag.bind(null,n,a,zt,Mu,Jm,t,ba,_r,Ls,Kn,s,2,-0,0),r);break e}Ag(n,a,zt,Mu,Jm,t,ba,_r,Ls,Kn,s,0,-0,0)}}break}while(!0);Ka(e)}function Ag(e,t,a,n,r,s,i,o,u,c,d,f,m,h){if(e.timeoutHandle=-1,f=t.subtreeFlags,(f&8192||(f&16785408)===16785408)&&(bo={stylesheets:null,count:0,unsuspend:IE},e0(t),f=HE(),f!==null)){e.cancelPendingCommit=f(Mg.bind(null,e,t,s,a,n,r,i,o,u,d,1,m,h)),Hn(e,s,i,!c);return}Mg(e,t,s,a,n,r,i,o,u)}function cE(e){for(var t=e;;){var a=t.tag;if((a===0||a===11||a===15)&&t.flags&16384&&(a=t.updateQueue,a!==null&&(a=a.stores,a!==null)))for(var n=0;n<a.length;n++){var r=a[n],s=r.getSnapshot;r=r.value;try{if(!aa(s(),r))return!1}catch{return!1}}if(a=t.child,t.subtreeFlags&16384&&a!==null)a.return=t,t=a;else{if(t===e)break;for(;t.sibling===null;){if(t.return===null||t.return===e)return!0;t=t.return}t.sibling.return=t.return,t=t.sibling}}return!0}function Hn(e,t,a,n){t&=~Jf,t&=~_r,e.suspendedLanes|=t,e.pingedLanes&=~t,n&&(e.warmLanes|=t),n=e.expirationTimes;for(var r=t;0<r;){var s=31-Wt(r),i=1<<s;n[s]=-1,r&=~i}a!==0&&fy(e,a,t)}function Wu(){return(we&6)===0?(Uo(0,!1),!1):!0}function Zf(){if(ue!==null){if($e===0)var e=ue.return;else e=ue,dn=Or=null,jf(e),ks=null,vo=0,e=ue;for(;e!==null;)zb(e.alternate,e),e=e.return;ue=null}}function Us(e,t){var a=e.timeoutHandle;a!==-1&&(e.timeoutHandle=-1,kE(a)),a=e.cancelPendingCommit,a!==null&&(e.cancelPendingCommit=null,a()),Zf(),ke=e,ue=a=fn(e.current,null),de=t,$e=0,Jt=null,Kn=!1,qs=ko(e,t),Yf=!1,Ls=ba=Jf=_r=ar=Ie=0,zt=io=null,Jm=!1,(t&8)!==0&&(t|=t&32);var n=e.entangledLanes;if(n!==0)for(e=e.entanglements,n&=t;0<n;){var r=31-Wt(n),s=1<<r;t|=e[r],n&=~s}return yn=t,Vu(),a}function o0(e,t){se=null,ne.H=ku,t===Mo||t===Yu?(t=ug(),$e=3):t===Hy?(t=ug(),$e=4):$e=t===Lb?8:t!==null&&typeof t=="object"&&typeof t.then=="function"?6:1,Jt=t,ue===null&&(Ie=1,Eu(e,ya(t,e.current)))}function l0(){var e=ne.H;return ne.H=ku,e===null?ku:e}function u0(){var e=ne.A;return ne.A=lE,e}function ef(){Ie=4,Kn||(de&4194048)!==de&&xa.current!==null||(qs=!0),(ar&134217727)===0&&(_r&134217727)===0||ke===null||Hn(ke,de,ba,!1)}function dm(e,t,a){var n=we;we|=2;var r=l0(),s=u0();(ke!==e||de!==t)&&(Mu=null,Us(e,t)),t=!1;var i=Ie;e:do try{if($e!==0&&ue!==null){var o=ue,u=Jt;switch($e){case 8:Zf(),i=6;break e;case 3:case 2:case 9:case 6:xa.current===null&&(t=!0);var c=$e;if($e=0,Jt=null,bs(e,o,u,c),a&&qs){i=0;break e}break;default:c=$e,$e=0,Jt=null,bs(e,o,u,c)}}dE(),i=Ie;break}catch(d){o0(e,d)}while(!0);return t&&e.shellSuspendCounter++,dn=Or=null,we=n,ne.H=r,ne.A=s,ue===null&&(ke=null,de=0,Vu()),i}function dE(){for(;ue!==null;)c0(ue)}function mE(e,t){var a=we;we|=2;var n=l0(),r=u0();ke!==e||de!==t?(Mu=null,Du=za()+500,Us(e,t)):qs=ko(e,t);e:do try{if($e!==0&&ue!==null){t=ue;var s=Jt;t:switch($e){case 1:$e=0,Jt=null,bs(e,t,s,1);break;case 2:case 9:if(lg(s)){$e=0,Jt=null,Dg(t);break}t=function(){$e!==2&&$e!==9||ke!==e||($e=7),Ka(e)},s.then(t,t);break e;case 3:$e=7;break e;case 4:$e=5;break e;case 7:lg(s)?($e=0,Jt=null,Dg(t)):($e=0,Jt=null,bs(e,t,s,7));break;case 5:var i=null;switch(ue.tag){case 26:i=ue.memoizedState;case 5:case 27:var o=ue;if(!i||E0(i)){$e=0,Jt=null;var u=o.sibling;if(u!==null)ue=u;else{var c=o.return;c!==null?(ue=c,ec(c)):ue=null}break t}}$e=0,Jt=null,bs(e,t,s,5);break;case 6:$e=0,Jt=null,bs(e,t,s,6);break;case 8:Zf(),Ie=6;break e;default:throw Error(j(462))}}fE();break}catch(d){o0(e,d)}while(!0);return dn=Or=null,ne.H=n,ne.A=r,we=a,ue!==null?0:(ke=null,de=0,Vu(),Ie)}function fE(){for(;ue!==null&&!LR();)c0(ue)}function c0(e){var t=Bb(e.alternate,e,yn);e.memoizedProps=e.pendingProps,t===null?ec(e):ue=t}function Dg(e){var t=e,a=t.alternate;switch(t.tag){case 15:case 0:t=_g(a,t,t.pendingProps,t.type,void 0,de);break;case 11:t=_g(a,t,t.pendingProps,t.type.render,t.ref,de);break;case 5:jf(t);default:zb(a,t),t=ue=zy(t,yn),t=Bb(a,t,yn)}e.memoizedProps=e.pendingProps,t===null?ec(e):ue=t}function bs(e,t,a,n){dn=Or=null,jf(t),ks=null,vo=0;var r=t.return;try{if(aE(e,r,t,a,de)){Ie=1,Eu(e,ya(a,e.current)),ue=null;return}}catch(s){if(r!==null)throw ue=r,s;Ie=1,Eu(e,ya(a,e.current)),ue=null;return}t.flags&32768?(ve||n===1?e=!0:qs||(de&536870912)!==0?e=!1:(Kn=e=!0,(n===2||n===9||n===3||n===6)&&(n=xa.current,n!==null&&n.tag===13&&(n.flags|=16384))),d0(t,e)):ec(t)}function ec(e){var t=e;do{if((t.flags&32768)!==0){d0(t,Kn);return}e=t.return;var a=rE(t.alternate,t,yn);if(a!==null){ue=a;return}if(t=t.sibling,t!==null){ue=t;return}ue=t=e}while(t!==null);Ie===0&&(Ie=5)}function d0(e,t){do{var a=sE(e.alternate,e);if(a!==null){a.flags&=32767,ue=a;return}if(a=e.return,a!==null&&(a.flags|=32768,a.subtreeFlags=0,a.deletions=null),!t&&(e=e.sibling,e!==null)){ue=e;return}ue=e=a}while(e!==null);Ie=6,ue=null}function Mg(e,t,a,n,r,s,i,o,u){e.cancelPendingCommit=null;do tc();while(gt!==0);if((we&6)!==0)throw Error(j(327));if(t!==null){if(t===e.current)throw Error(j(177));if(s=t.lanes|t.childLanes,s|=_f,HR(e,a,s,i,o,u),e===ke&&(ue=ke=null,de=0),Ps=t,Jn=e,Rs=a,Xm=s,Zm=r,r0=n,(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?(e.callbackNode=null,e.callbackPriority=0,gE(vu,function(){return v0(!0),null})):(e.callbackNode=null,e.callbackPriority=0),n=(t.flags&13878)!==0,(t.subtreeFlags&13878)!==0||n){n=ne.T,ne.T=null,r=ge.p,ge.p=2,i=we,we|=4;try{iE(e,t,a)}finally{we=i,ge.p=r,ne.T=n}}gt=1,m0(),f0(),p0()}}function m0(){if(gt===1){gt=0;var e=Jn,t=Ps,a=(t.flags&13878)!==0;if((t.subtreeFlags&13878)!==0||a){a=ne.T,ne.T=null;var n=ge.p;ge.p=2;var r=we;we|=4;try{Xb(t,e);var s=rf,i=My(e.containerInfo),o=s.focusedElem,u=s.selectionRange;if(i!==o&&o&&o.ownerDocument&&Dy(o.ownerDocument.documentElement,o)){if(u!==null&&Nf(o)){var c=u.start,d=u.end;if(d===void 0&&(d=c),"selectionStart"in o)o.selectionStart=c,o.selectionEnd=Math.min(d,o.value.length);else{var f=o.ownerDocument||document,m=f&&f.defaultView||window;if(m.getSelection){var h=m.getSelection(),b=o.textContent.length,y=Math.min(u.start,b),w=u.end===void 0?y:Math.min(u.end,b);!h.extend&&y>w&&(i=w,w=y,y=i);var g=eg(o,y),v=eg(o,w);if(g&&v&&(h.rangeCount!==1||h.anchorNode!==g.node||h.anchorOffset!==g.offset||h.focusNode!==v.node||h.focusOffset!==v.offset)){var x=f.createRange();x.setStart(g.node,g.offset),h.removeAllRanges(),y>w?(h.addRange(x),h.extend(v.node,v.offset)):(x.setEnd(v.node,v.offset),h.addRange(x))}}}}for(f=[],h=o;h=h.parentNode;)h.nodeType===1&&f.push({element:h,left:h.scrollLeft,top:h.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<f.length;o++){var $=f[o];$.element.scrollLeft=$.left,$.element.scrollTop=$.top}}zu=!!nf,rf=nf=null}finally{we=r,ge.p=n,ne.T=a}}e.current=t,gt=2}}function f0(){if(gt===2){gt=0;var e=Jn,t=Ps,a=(t.flags&8772)!==0;if((t.subtreeFlags&8772)!==0||a){a=ne.T,ne.T=null;var n=ge.p;ge.p=2;var r=we;we|=4;try{Vb(e,t.alternate,t)}finally{we=r,ge.p=n,ne.T=a}}gt=3}}function p0(){if(gt===4||gt===3){gt=0,PR();var e=Jn,t=Ps,a=Rs,n=r0;(t.subtreeFlags&10256)!==0||(t.flags&10256)!==0?gt=5:(gt=0,Ps=Jn=null,h0(e,e.pendingLanes));var r=e.pendingLanes;if(r===0&&(Yn=null),gf(a),t=t.stateNode,Zt&&typeof Zt.onCommitFiberRoot=="function")try{Zt.onCommitFiberRoot(_o,t,void 0,(t.current.flags&128)===128)}catch{}if(n!==null){t=ne.T,r=ge.p,ge.p=2,ne.T=null;try{for(var s=e.onRecoverableError,i=0;i<n.length;i++){var o=n[i];s(o.value,{componentStack:o.stack})}}finally{ne.T=t,ge.p=r}}(Rs&3)!==0&&tc(),Ka(e),r=e.pendingLanes,(a&4194090)!==0&&(r&42)!==0?e===Wm?oo++:(oo=0,Wm=e):oo=0,Uo(0,!1)}}function h0(e,t){(e.pooledCacheLanes&=t)===0&&(t=e.pooledCache,t!=null&&(e.pooledCache=null,Do(t)))}function tc(e){return m0(),f0(),p0(),v0(e)}function v0(){if(gt!==5)return!1;var e=Jn,t=Xm;Xm=0;var a=gf(Rs),n=ne.T,r=ge.p;try{ge.p=32>a?32:a,ne.T=null,a=Zm,Zm=null;var s=Jn,i=Rs;if(gt=0,Ps=Jn=null,Rs=0,(we&6)!==0)throw Error(j(331));var o=we;if(we|=4,a0(s.current),Wb(s,s.current,i,a),we=o,Uo(0,!1),Zt&&typeof Zt.onPostCommitFiberRoot=="function")try{Zt.onPostCommitFiberRoot(_o,s)}catch{}return!0}finally{ge.p=r,ne.T=n,h0(e,t)}}function Og(e,t,a){t=ya(a,t),t=Qm(e.stateNode,t,2),e=Gn(e,t,2),e!==null&&(Ro(e,2),Ka(e))}function _e(e,t,a){if(e.tag===3)Og(e,e,a);else for(;t!==null;){if(t.tag===3){Og(t,e,a);break}else if(t.tag===1){var n=t.stateNode;if(typeof t.type.getDerivedStateFromError=="function"||typeof n.componentDidCatch=="function"&&(Yn===null||!Yn.has(n))){e=ya(a,e),a=Mb(2),n=Gn(t,a,2),n!==null&&(Ob(a,n,t,e),Ro(n,2),Ka(n));break}}t=t.return}}function mm(e,t,a){var n=e.pingCache;if(n===null){n=e.pingCache=new uE;var r=new Set;n.set(t,r)}else r=n.get(t),r===void 0&&(r=new Set,n.set(t,r));r.has(a)||(Yf=!0,r.add(a),e=pE.bind(null,e,t,a),t.then(e,e))}function pE(e,t,a){var n=e.pingCache;n!==null&&n.delete(t),e.pingedLanes|=e.suspendedLanes&a,e.warmLanes&=~a,ke===e&&(de&a)===a&&(Ie===4||Ie===3&&(de&62914560)===de&&300>za()-Xf?(we&2)===0&&Us(e,0):Jf|=a,Ls===de&&(Ls=0)),Ka(e)}function g0(e,t){t===0&&(t=my()),e=zs(e,t),e!==null&&(Ro(e,t),Ka(e))}function hE(e){var t=e.memoizedState,a=0;t!==null&&(a=t.retryLane),g0(e,a)}function vE(e,t){var a=0;switch(e.tag){case 13:var n=e.stateNode,r=e.memoizedState;r!==null&&(a=r.retryLane);break;case 19:n=e.stateNode;break;case 22:n=e.stateNode._retryCache;break;default:throw Error(j(314))}n!==null&&n.delete(t),g0(e,a)}function gE(e,t){return hf(e,t)}var Ou=null,os=null,tf=!1,Lu=!1,fm=!1,kr=0;function Ka(e){e!==os&&e.next===null&&(os===null?Ou=os=e:os=os.next=e),Lu=!0,tf||(tf=!0,bE())}function Uo(e,t){if(!fm&&Lu){fm=!0;do for(var a=!1,n=Ou;n!==null;){if(!t)if(e!==0){var r=n.pendingLanes;if(r===0)var s=0;else{var i=n.suspendedLanes,o=n.pingedLanes;s=(1<<31-Wt(42|e)+1)-1,s&=r&~(i&~o),s=s&201326741?s&201326741|1:s?s|2:0}s!==0&&(a=!0,Lg(n,s))}else s=de,s=Iu(n,n===ke?s:0,n.cancelPendingCommit!==null||n.timeoutHandle!==-1),(s&3)===0||ko(n,s)||(a=!0,Lg(n,s));n=n.next}while(a);fm=!1}}function yE(){y0()}function y0(){Lu=tf=!1;var e=0;kr!==0&&(_E()&&(e=kr),kr=0);for(var t=za(),a=null,n=Ou;n!==null;){var r=n.next,s=b0(n,t);s===0?(n.next=null,a===null?Ou=r:a.next=r,r===null&&(os=a)):(a=n,(e!==0||(s&3)!==0)&&(Lu=!0)),n=r}Uo(e,!1)}function b0(e,t){for(var a=e.suspendedLanes,n=e.pingedLanes,r=e.expirationTimes,s=e.pendingLanes&-62914561;0<s;){var i=31-Wt(s),o=1<<i,u=r[i];u===-1?((o&a)===0||(o&n)!==0)&&(r[i]=KR(o,t)):u<=t&&(e.expiredLanes|=o),s&=~o}if(t=ke,a=de,a=Iu(e,e===t?a:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n=e.callbackNode,a===0||e===t&&($e===2||$e===9)||e.cancelPendingCommit!==null)return n!==null&&n!==null&&Bd(n),e.callbackNode=null,e.callbackPriority=0;if((a&3)===0||ko(e,a)){if(t=a&-a,t===e.callbackPriority)return t;switch(n!==null&&Bd(n),gf(a)){case 2:case 8:a=uy;break;case 32:a=vu;break;case 268435456:a=cy;break;default:a=vu}return n=x0.bind(null,e),a=hf(a,n),e.callbackPriority=t,e.callbackNode=a,t}return n!==null&&n!==null&&Bd(n),e.callbackPriority=2,e.callbackNode=null,2}function x0(e,t){if(gt!==0&&gt!==5)return e.callbackNode=null,e.callbackPriority=0,null;var a=e.callbackNode;if(tc(!0)&&e.callbackNode!==a)return null;var n=de;return n=Iu(e,e===ke?n:0,e.cancelPendingCommit!==null||e.timeoutHandle!==-1),n===0?null:(i0(e,n,t),b0(e,za()),e.callbackNode!=null&&e.callbackNode===a?x0.bind(null,e):null)}function Lg(e,t){if(tc())return null;i0(e,t,!0)}function bE(){RE(function(){(we&6)!==0?hf(ly,yE):y0()})}function Wf(){return kr===0&&(kr=dy()),kr}function Pg(e){return e==null||typeof e=="symbol"||typeof e=="boolean"?null:typeof e=="function"?e:tu(""+e)}function Ug(e,t){var a=t.ownerDocument.createElement("input");return a.name=t.name,a.value=t.value,e.id&&a.setAttribute("form",e.id),t.parentNode.insertBefore(a,t),e=new FormData(e),a.parentNode.removeChild(a),e}function xE(e,t,a,n,r){if(t==="submit"&&a&&a.stateNode===r){var s=Pg((r[qt]||null).action),i=n.submitter;i&&(t=(t=i[qt]||null)?Pg(t.formAction):i.getAttribute("formAction"),t!==null&&(s=t,i=null));var o=new Ku("action","action",null,n,r);e.push({event:o,listeners:[{instance:null,listener:function(){if(n.defaultPrevented){if(kr!==0){var u=i?Ug(r,i):new FormData(r);Km(a,{pending:!0,data:u,method:r.method,action:s},null,u)}}else typeof s=="function"&&(o.preventDefault(),u=i?Ug(r,i):new FormData(r),Km(a,{pending:!0,data:u,method:r.method,action:s},s,u))},currentTarget:r}]})}}for(Yl=0;Yl<Dm.length;Yl++)Jl=Dm[Yl],jg=Jl.toLowerCase(),Fg=Jl[0].toUpperCase()+Jl.slice(1),Ra(jg,"on"+Fg);var Jl,jg,Fg,Yl;Ra(Ly,"onAnimationEnd");Ra(Py,"onAnimationIteration");Ra(Uy,"onAnimationStart");Ra("dblclick","onDoubleClick");Ra("focusin","onFocus");Ra("focusout","onBlur");Ra(FC,"onTransitionRun");Ra(BC,"onTransitionStart");Ra(zC,"onTransitionCancel");Ra(jy,"onTransitionEnd");Es("onMouseEnter",["mouseout","mouseover"]);Es("onMouseLeave",["mouseout","mouseover"]);Es("onPointerEnter",["pointerout","pointerover"]);Es("onPointerLeave",["pointerout","pointerover"]);Ar("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Ar("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Ar("onBeforeInput",["compositionend","keypress","textInput","paste"]);Ar("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Ar("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Ar("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var go="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),$E=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(go));function $0(e,t){t=(t&4)!==0;for(var a=0;a<e.length;a++){var n=e[a],r=n.event;n=n.listeners;e:{var s=void 0;if(t)for(var i=n.length-1;0<=i;i--){var o=n[i],u=o.instance,c=o.currentTarget;if(o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Cu(d)}r.currentTarget=null,s=u}else for(i=0;i<n.length;i++){if(o=n[i],u=o.instance,c=o.currentTarget,o=o.listener,u!==s&&r.isPropagationStopped())break e;s=o,r.currentTarget=c;try{s(r)}catch(d){Cu(d)}r.currentTarget=null,s=u}}}}function le(e,t){var a=t[_m];a===void 0&&(a=t[_m]=new Set);var n=e+"__bubble";a.has(n)||(w0(t,e,2,!1),a.add(n))}function pm(e,t,a){var n=0;t&&(n|=4),w0(a,e,n,t)}var Xl="_reactListening"+Math.random().toString(36).slice(2);function ep(e){if(!e[Xl]){e[Xl]=!0,vy.forEach(function(a){a!=="selectionchange"&&($E.has(a)||pm(a,!1,e),pm(a,!0,e))});var t=e.nodeType===9?e:e.ownerDocument;t===null||t[Xl]||(t[Xl]=!0,pm("selectionchange",!1,t))}}function w0(e,t,a,n){switch(O0(t)){case 2:var r=GE;break;case 8:r=YE;break;default:r=rp}a=r.bind(null,t,a,e),r=void 0,!Em||t!=="touchstart"&&t!=="touchmove"&&t!=="wheel"||(r=!0),n?r!==void 0?e.addEventListener(t,a,{capture:!0,passive:r}):e.addEventListener(t,a,!0):r!==void 0?e.addEventListener(t,a,{passive:r}):e.addEventListener(t,a,!1)}function hm(e,t,a,n,r){var s=n;if((t&1)===0&&(t&2)===0&&n!==null)e:for(;;){if(n===null)return;var i=n.tag;if(i===3||i===4){var o=n.stateNode.containerInfo;if(o===r)break;if(i===4)for(i=n.return;i!==null;){var u=i.tag;if((u===3||u===4)&&i.stateNode.containerInfo===r)return;i=i.return}for(;o!==null;){if(i=cs(o),i===null)return;if(u=i.tag,u===5||u===6||u===26||u===27){n=s=i;continue e}o=o.parentNode}}n=n.return}Ny(function(){var c=s,d=xf(a),f=[];e:{var m=Fy.get(e);if(m!==void 0){var h=Ku,b=e;switch(e){case"keypress":if(nu(a)===0)break e;case"keydown":case"keyup":h=gC;break;case"focusin":b="focus",h=Gd;break;case"focusout":b="blur",h=Gd;break;case"beforeblur":case"afterblur":h=Gd;break;case"click":if(a.button===2)break e;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":h=Hv;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":h=sC;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":h=xC;break;case Ly:case Py:case Uy:h=lC;break;case jy:h=wC;break;case"scroll":case"scrollend":h=nC;break;case"wheel":h=NC;break;case"copy":case"cut":case"paste":h=cC;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":h=Vv;break;case"toggle":case"beforetoggle":h=kC}var y=(t&4)!==0,w=!y&&(e==="scroll"||e==="scrollend"),g=y?m!==null?m+"Capture":null:m;y=[];for(var v=c,x;v!==null;){var $=v;if(x=$.stateNode,$=$.tag,$!==5&&$!==26&&$!==27||x===null||g===null||($=co(v,g),$!=null&&y.push(yo(v,$,x))),w)break;v=v.return}0<y.length&&(m=new h(m,b,null,a,d),f.push({event:m,listeners:y}))}}if((t&7)===0){e:{if(m=e==="mouseover"||e==="pointerover",h=e==="mouseout"||e==="pointerout",m&&a!==Cm&&(b=a.relatedTarget||a.fromElement)&&(cs(b)||b[Fs]))break e;if((h||m)&&(m=d.window===d?d:(m=d.ownerDocument)?m.defaultView||m.parentWindow:window,h?(b=a.relatedTarget||a.toElement,h=c,b=b?cs(b):null,b!==null&&(w=No(b),y=b.tag,b!==w||y!==5&&y!==27&&y!==6)&&(b=null)):(h=null,b=c),h!==b)){if(y=Hv,$="onMouseLeave",g="onMouseEnter",v="mouse",(e==="pointerout"||e==="pointerover")&&(y=Vv,$="onPointerLeave",g="onPointerEnter",v="pointer"),w=h==null?m:Vi(h),x=b==null?m:Vi(b),m=new y($,v+"leave",h,a,d),m.target=w,m.relatedTarget=x,$=null,cs(d)===c&&(y=new y(g,v+"enter",b,a,d),y.target=x,y.relatedTarget=w,$=y),w=$,h&&b)t:{for(y=h,g=b,v=0,x=y;x;x=rs(x))v++;for(x=0,$=g;$;$=rs($))x++;for(;0<v-x;)y=rs(y),v--;for(;0<x-v;)g=rs(g),x--;for(;v--;){if(y===g||g!==null&&y===g.alternate)break t;y=rs(y),g=rs(g)}y=null}else y=null;h!==null&&Bg(f,m,h,y,!1),b!==null&&w!==null&&Bg(f,w,b,y,!0)}}e:{if(m=c?Vi(c):window,h=m.nodeName&&m.nodeName.toLowerCase(),h==="select"||h==="input"&&m.type==="file")var S=Xv;else if(Jv(m))if(Ty)S=PC;else{S=OC;var C=MC}else h=m.nodeName,!h||h.toLowerCase()!=="input"||m.type!=="checkbox"&&m.type!=="radio"?c&&bf(c.elementType)&&(S=Xv):S=LC;if(S&&(S=S(e,c))){Ey(f,S,a,d);break e}C&&C(e,m,c),e==="focusout"&&c&&m.type==="number"&&c.memoizedProps.value!=null&&Rm(m,"number",m.value)}switch(C=c?Vi(c):window,e){case"focusin":(Jv(C)||C.contentEditable==="true")&&(fs=C,Tm=c,Xi=null);break;case"focusout":Xi=Tm=fs=null;break;case"mousedown":Am=!0;break;case"contextmenu":case"mouseup":case"dragend":Am=!1,tg(f,a,d);break;case"selectionchange":if(jC)break;case"keydown":case"keyup":tg(f,a,d)}var _;if(Sf)e:{switch(e){case"compositionstart":var T="onCompositionStart";break e;case"compositionend":T="onCompositionEnd";break e;case"compositionupdate":T="onCompositionUpdate";break e}T=void 0}else ms?Ry(e,a)&&(T="onCompositionEnd"):e==="keydown"&&a.keyCode===229&&(T="onCompositionStart");T&&(ky&&a.locale!=="ko"&&(ms||T!=="onCompositionStart"?T==="onCompositionEnd"&&ms&&(_=_y()):(In=d,$f="value"in In?In.value:In.textContent,ms=!0)),C=Pu(c,T),0<C.length&&(T=new Qv(T,e,null,a,d),f.push({event:T,listeners:C}),_?T.data=_:(_=Cy(a),_!==null&&(T.data=_)))),(_=CC?EC(e,a):TC(e,a))&&(T=Pu(c,"onBeforeInput"),0<T.length&&(C=new Qv("onBeforeInput","beforeinput",null,a,d),f.push({event:C,listeners:T}),C.data=_)),xE(f,e,c,a,d)}$0(f,t)})}function yo(e,t,a){return{instance:e,listener:t,currentTarget:a}}function Pu(e,t){for(var a=t+"Capture",n=[];e!==null;){var r=e,s=r.stateNode;if(r=r.tag,r!==5&&r!==26&&r!==27||s===null||(r=co(e,a),r!=null&&n.unshift(yo(e,r,s)),r=co(e,t),r!=null&&n.push(yo(e,r,s))),e.tag===3)return n;e=e.return}return[]}function rs(e){if(e===null)return null;do e=e.return;while(e&&e.tag!==5&&e.tag!==27);return e||null}function Bg(e,t,a,n,r){for(var s=t._reactName,i=[];a!==null&&a!==n;){var o=a,u=o.alternate,c=o.stateNode;if(o=o.tag,u!==null&&u===n)break;o!==5&&o!==26&&o!==27||c===null||(u=c,r?(c=co(a,s),c!=null&&i.unshift(yo(a,c,u))):r||(c=co(a,s),c!=null&&i.push(yo(a,c,u)))),a=a.return}i.length!==0&&e.push({event:t,listeners:i})}var wE=/\r\n?/g,SE=/\u0000|\uFFFD/g;function zg(e){return(typeof e=="string"?e:""+e).replace(wE,`
`).replace(SE,"")}function S0(e,t){return t=zg(t),zg(e)===t}function ac(){}function Se(e,t,a,n,r,s){switch(a){case"children":typeof n=="string"?t==="body"||t==="textarea"&&n===""||Ts(e,n):(typeof n=="number"||typeof n=="bigint")&&t!=="body"&&Ts(e,""+n);break;case"className":Bl(e,"class",n);break;case"tabIndex":Bl(e,"tabindex",n);break;case"dir":case"role":case"viewBox":case"width":case"height":Bl(e,a,n);break;case"style":Sy(e,n,s);break;case"data":if(t!=="object"){Bl(e,"data",n);break}case"src":case"href":if(n===""&&(t!=="a"||a!=="href")){e.removeAttribute(a);break}if(n==null||typeof n=="function"||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=tu(""+n),e.setAttribute(a,n);break;case"action":case"formAction":if(typeof n=="function"){e.setAttribute(a,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof s=="function"&&(a==="formAction"?(t!=="input"&&Se(e,t,"name",r.name,r,null),Se(e,t,"formEncType",r.formEncType,r,null),Se(e,t,"formMethod",r.formMethod,r,null),Se(e,t,"formTarget",r.formTarget,r,null)):(Se(e,t,"encType",r.encType,r,null),Se(e,t,"method",r.method,r,null),Se(e,t,"target",r.target,r,null)));if(n==null||typeof n=="symbol"||typeof n=="boolean"){e.removeAttribute(a);break}n=tu(""+n),e.setAttribute(a,n);break;case"onClick":n!=null&&(e.onclick=ac);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"multiple":e.multiple=n&&typeof n!="function"&&typeof n!="symbol";break;case"muted":e.muted=n&&typeof n!="function"&&typeof n!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(n==null||typeof n=="function"||typeof n=="boolean"||typeof n=="symbol"){e.removeAttribute("xlink:href");break}a=tu(""+n),e.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",a);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""+n):e.removeAttribute(a);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":n&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,""):e.removeAttribute(a);break;case"capture":case"download":n===!0?e.setAttribute(a,""):n!==!1&&n!=null&&typeof n!="function"&&typeof n!="symbol"?e.setAttribute(a,n):e.removeAttribute(a);break;case"cols":case"rows":case"size":case"span":n!=null&&typeof n!="function"&&typeof n!="symbol"&&!isNaN(n)&&1<=n?e.setAttribute(a,n):e.removeAttribute(a);break;case"rowSpan":case"start":n==null||typeof n=="function"||typeof n=="symbol"||isNaN(n)?e.removeAttribute(a):e.setAttribute(a,n);break;case"popover":le("beforetoggle",e),le("toggle",e),eu(e,"popover",n);break;case"xlinkActuate":nn(e,"http://www.w3.org/1999/xlink","xlink:actuate",n);break;case"xlinkArcrole":nn(e,"http://www.w3.org/1999/xlink","xlink:arcrole",n);break;case"xlinkRole":nn(e,"http://www.w3.org/1999/xlink","xlink:role",n);break;case"xlinkShow":nn(e,"http://www.w3.org/1999/xlink","xlink:show",n);break;case"xlinkTitle":nn(e,"http://www.w3.org/1999/xlink","xlink:title",n);break;case"xlinkType":nn(e,"http://www.w3.org/1999/xlink","xlink:type",n);break;case"xmlBase":nn(e,"http://www.w3.org/XML/1998/namespace","xml:base",n);break;case"xmlLang":nn(e,"http://www.w3.org/XML/1998/namespace","xml:lang",n);break;case"xmlSpace":nn(e,"http://www.w3.org/XML/1998/namespace","xml:space",n);break;case"is":eu(e,"is",n);break;case"innerText":case"textContent":break;default:(!(2<a.length)||a[0]!=="o"&&a[0]!=="O"||a[1]!=="n"&&a[1]!=="N")&&(a=tC.get(a)||a,eu(e,a,n))}}function af(e,t,a,n,r,s){switch(a){case"style":Sy(e,n,s);break;case"dangerouslySetInnerHTML":if(n!=null){if(typeof n!="object"||!("__html"in n))throw Error(j(61));if(a=n.__html,a!=null){if(r.children!=null)throw Error(j(60));e.innerHTML=a}}break;case"children":typeof n=="string"?Ts(e,n):(typeof n=="number"||typeof n=="bigint")&&Ts(e,""+n);break;case"onScroll":n!=null&&le("scroll",e);break;case"onScrollEnd":n!=null&&le("scrollend",e);break;case"onClick":n!=null&&(e.onclick=ac);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!gy.hasOwnProperty(a))e:{if(a[0]==="o"&&a[1]==="n"&&(r=a.endsWith("Capture"),t=a.slice(2,r?a.length-7:void 0),s=e[qt]||null,s=s!=null?s[a]:null,typeof s=="function"&&e.removeEventListener(t,s,r),typeof n=="function")){typeof s!="function"&&s!==null&&(a in e?e[a]=null:e.hasAttribute(a)&&e.removeAttribute(a)),e.addEventListener(t,n,r);break e}a in e?e[a]=n:n===!0?e.setAttribute(a,""):eu(e,a,n)}}}function yt(e,t,a){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":le("error",e),le("load",e);var n=!1,r=!1,s;for(s in a)if(a.hasOwnProperty(s)){var i=a[s];if(i!=null)switch(s){case"src":n=!0;break;case"srcSet":r=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Se(e,t,s,i,a,null)}}r&&Se(e,t,"srcSet",a.srcSet,a,null),n&&Se(e,t,"src",a.src,a,null);return;case"input":le("invalid",e);var o=s=i=r=null,u=null,c=null;for(n in a)if(a.hasOwnProperty(n)){var d=a[n];if(d!=null)switch(n){case"name":r=d;break;case"type":i=d;break;case"checked":u=d;break;case"defaultChecked":c=d;break;case"value":s=d;break;case"defaultValue":o=d;break;case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(j(137,t));break;default:Se(e,t,n,d,a,null)}}xy(e,s,o,u,c,i,r,!1),gu(e);return;case"select":le("invalid",e),n=i=s=null;for(r in a)if(a.hasOwnProperty(r)&&(o=a[r],o!=null))switch(r){case"value":s=o;break;case"defaultValue":i=o;break;case"multiple":n=o;default:Se(e,t,r,o,a,null)}t=s,a=i,e.multiple=!!n,t!=null?$s(e,!!n,t,!1):a!=null&&$s(e,!!n,a,!0);return;case"textarea":le("invalid",e),s=r=n=null;for(i in a)if(a.hasOwnProperty(i)&&(o=a[i],o!=null))switch(i){case"value":n=o;break;case"defaultValue":r=o;break;case"children":s=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(j(91));break;default:Se(e,t,i,o,a,null)}wy(e,n,r,s),gu(e);return;case"option":for(u in a)if(a.hasOwnProperty(u)&&(n=a[u],n!=null))switch(u){case"selected":e.selected=n&&typeof n!="function"&&typeof n!="symbol";break;default:Se(e,t,u,n,a,null)}return;case"dialog":le("beforetoggle",e),le("toggle",e),le("cancel",e),le("close",e);break;case"iframe":case"object":le("load",e);break;case"video":case"audio":for(n=0;n<go.length;n++)le(go[n],e);break;case"image":le("error",e),le("load",e);break;case"details":le("toggle",e);break;case"embed":case"source":case"link":le("error",e),le("load",e);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(c in a)if(a.hasOwnProperty(c)&&(n=a[c],n!=null))switch(c){case"children":case"dangerouslySetInnerHTML":throw Error(j(137,t));default:Se(e,t,c,n,a,null)}return;default:if(bf(t)){for(d in a)a.hasOwnProperty(d)&&(n=a[d],n!==void 0&&af(e,t,d,n,a,void 0));return}}for(o in a)a.hasOwnProperty(o)&&(n=a[o],n!=null&&Se(e,t,o,n,a,null))}function NE(e,t,a,n){switch(t){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var r=null,s=null,i=null,o=null,u=null,c=null,d=null;for(h in a){var f=a[h];if(a.hasOwnProperty(h)&&f!=null)switch(h){case"checked":break;case"value":break;case"defaultValue":u=f;default:n.hasOwnProperty(h)||Se(e,t,h,null,n,f)}}for(var m in n){var h=n[m];if(f=a[m],n.hasOwnProperty(m)&&(h!=null||f!=null))switch(m){case"type":s=h;break;case"name":r=h;break;case"checked":c=h;break;case"defaultChecked":d=h;break;case"value":i=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(j(137,t));break;default:h!==f&&Se(e,t,m,h,n,f)}}km(e,i,o,u,c,d,s,r);return;case"select":h=i=o=m=null;for(s in a)if(u=a[s],a.hasOwnProperty(s)&&u!=null)switch(s){case"value":break;case"multiple":h=u;default:n.hasOwnProperty(s)||Se(e,t,s,null,n,u)}for(r in n)if(s=n[r],u=a[r],n.hasOwnProperty(r)&&(s!=null||u!=null))switch(r){case"value":m=s;break;case"defaultValue":o=s;break;case"multiple":i=s;default:s!==u&&Se(e,t,r,s,n,u)}t=o,a=i,n=h,m!=null?$s(e,!!a,m,!1):!!n!=!!a&&(t!=null?$s(e,!!a,t,!0):$s(e,!!a,a?[]:"",!1));return;case"textarea":h=m=null;for(o in a)if(r=a[o],a.hasOwnProperty(o)&&r!=null&&!n.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:Se(e,t,o,null,n,r)}for(i in n)if(r=n[i],s=a[i],n.hasOwnProperty(i)&&(r!=null||s!=null))switch(i){case"value":m=r;break;case"defaultValue":h=r;break;case"children":break;case"dangerouslySetInnerHTML":if(r!=null)throw Error(j(91));break;default:r!==s&&Se(e,t,i,r,n,s)}$y(e,m,h);return;case"option":for(var b in a)if(m=a[b],a.hasOwnProperty(b)&&m!=null&&!n.hasOwnProperty(b))switch(b){case"selected":e.selected=!1;break;default:Se(e,t,b,null,n,m)}for(u in n)if(m=n[u],h=a[u],n.hasOwnProperty(u)&&m!==h&&(m!=null||h!=null))switch(u){case"selected":e.selected=m&&typeof m!="function"&&typeof m!="symbol";break;default:Se(e,t,u,m,n,h)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var y in a)m=a[y],a.hasOwnProperty(y)&&m!=null&&!n.hasOwnProperty(y)&&Se(e,t,y,null,n,m);for(c in n)if(m=n[c],h=a[c],n.hasOwnProperty(c)&&m!==h&&(m!=null||h!=null))switch(c){case"children":case"dangerouslySetInnerHTML":if(m!=null)throw Error(j(137,t));break;default:Se(e,t,c,m,n,h)}return;default:if(bf(t)){for(var w in a)m=a[w],a.hasOwnProperty(w)&&m!==void 0&&!n.hasOwnProperty(w)&&af(e,t,w,void 0,n,m);for(d in n)m=n[d],h=a[d],!n.hasOwnProperty(d)||m===h||m===void 0&&h===void 0||af(e,t,d,m,n,h);return}}for(var g in a)m=a[g],a.hasOwnProperty(g)&&m!=null&&!n.hasOwnProperty(g)&&Se(e,t,g,null,n,m);for(f in n)m=n[f],h=a[f],!n.hasOwnProperty(f)||m===h||m==null&&h==null||Se(e,t,f,m,n,h)}var nf=null,rf=null;function Uu(e){return e.nodeType===9?e:e.ownerDocument}function qg(e){switch(e){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function N0(e,t){if(e===0)switch(t){case"svg":return 1;case"math":return 2;default:return 0}return e===1&&t==="foreignObject"?0:e}function sf(e,t){return e==="textarea"||e==="noscript"||typeof t.children=="string"||typeof t.children=="number"||typeof t.children=="bigint"||typeof t.dangerouslySetInnerHTML=="object"&&t.dangerouslySetInnerHTML!==null&&t.dangerouslySetInnerHTML.__html!=null}var vm=null;function _E(){var e=window.event;return e&&e.type==="popstate"?e===vm?!1:(vm=e,!0):(vm=null,!1)}var _0=typeof setTimeout=="function"?setTimeout:void 0,kE=typeof clearTimeout=="function"?clearTimeout:void 0,Ig=typeof Promise=="function"?Promise:void 0,RE=typeof queueMicrotask=="function"?queueMicrotask:typeof Ig<"u"?function(e){return Ig.resolve(null).then(e).catch(CE)}:_0;function CE(e){setTimeout(function(){throw e})}function rr(e){return e==="head"}function Kg(e,t){var a=t,n=0,r=0;do{var s=a.nextSibling;if(e.removeChild(a),s&&s.nodeType===8)if(a=s.data,a==="/$"){if(0<n&&8>n){a=n;var i=e.ownerDocument;if(a&1&&lo(i.documentElement),a&2&&lo(i.body),a&4)for(a=i.head,lo(a),i=a.firstChild;i;){var o=i.nextSibling,u=i.nodeName;i[Co]||u==="SCRIPT"||u==="STYLE"||u==="LINK"&&i.rel.toLowerCase()==="stylesheet"||a.removeChild(i),i=o}}if(r===0){e.removeChild(s),So(t);return}r--}else a==="$"||a==="$?"||a==="$!"?r++:n=a.charCodeAt(0)-48;else n=0;a=s}while(a);So(t)}function of(e){var t=e.firstChild;for(t&&t.nodeType===10&&(t=t.nextSibling);t;){var a=t;switch(t=t.nextSibling,a.nodeName){case"HTML":case"HEAD":case"BODY":of(a),yf(a);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(a.rel.toLowerCase()==="stylesheet")continue}e.removeChild(a)}}function EE(e,t,a,n){for(;e.nodeType===1;){var r=a;if(e.nodeName.toLowerCase()!==t.toLowerCase()){if(!n&&(e.nodeName!=="INPUT"||e.type!=="hidden"))break}else if(n){if(!e[Co])switch(t){case"meta":if(!e.hasAttribute("itemprop"))break;return e;case"link":if(s=e.getAttribute("rel"),s==="stylesheet"&&e.hasAttribute("data-precedence"))break;if(s!==r.rel||e.getAttribute("href")!==(r.href==null||r.href===""?null:r.href)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin)||e.getAttribute("title")!==(r.title==null?null:r.title))break;return e;case"style":if(e.hasAttribute("data-precedence"))break;return e;case"script":if(s=e.getAttribute("src"),(s!==(r.src==null?null:r.src)||e.getAttribute("type")!==(r.type==null?null:r.type)||e.getAttribute("crossorigin")!==(r.crossOrigin==null?null:r.crossOrigin))&&s&&e.hasAttribute("async")&&!e.hasAttribute("itemprop"))break;return e;default:return e}}else if(t==="input"&&e.type==="hidden"){var s=r.name==null?null:""+r.name;if(r.type==="hidden"&&e.getAttribute("name")===s)return e}else return e;if(e=ka(e.nextSibling),e===null)break}return null}function TE(e,t,a){if(t==="")return null;for(;e.nodeType!==3;)if((e.nodeType!==1||e.nodeName!=="INPUT"||e.type!=="hidden")&&!a||(e=ka(e.nextSibling),e===null))return null;return e}function lf(e){return e.data==="$!"||e.data==="$?"&&e.ownerDocument.readyState==="complete"}function AE(e,t){var a=e.ownerDocument;if(e.data!=="$?"||a.readyState==="complete")t();else{var n=function(){t(),a.removeEventListener("DOMContentLoaded",n)};a.addEventListener("DOMContentLoaded",n),e._reactRetry=n}}function ka(e){for(;e!=null;e=e.nextSibling){var t=e.nodeType;if(t===1||t===3)break;if(t===8){if(t=e.data,t==="$"||t==="$!"||t==="$?"||t==="F!"||t==="F")break;if(t==="/$")return null}}return e}var uf=null;function Hg(e){e=e.previousSibling;for(var t=0;e;){if(e.nodeType===8){var a=e.data;if(a==="$"||a==="$!"||a==="$?"){if(t===0)return e;t--}else a==="/$"&&t++}e=e.previousSibling}return null}function k0(e,t,a){switch(t=Uu(a),e){case"html":if(e=t.documentElement,!e)throw Error(j(452));return e;case"head":if(e=t.head,!e)throw Error(j(453));return e;case"body":if(e=t.body,!e)throw Error(j(454));return e;default:throw Error(j(451))}}function lo(e){for(var t=e.attributes;t.length;)e.removeAttributeNode(t[0]);yf(e)}var $a=new Map,Qg=new Set;function ju(e){return typeof e.getRootNode=="function"?e.getRootNode():e.nodeType===9?e:e.ownerDocument}var bn=ge.d;ge.d={f:DE,r:ME,D:OE,C:LE,L:PE,m:UE,X:FE,S:jE,M:BE};function DE(){var e=bn.f(),t=Wu();return e||t}function ME(e){var t=Bs(e);t!==null&&t.tag===5&&t.type==="form"?bb(t):bn.r(e)}var Is=typeof document>"u"?null:document;function R0(e,t,a){var n=Is;if(n&&typeof t=="string"&&t){var r=ga(t);r='link[rel="'+e+'"][href="'+r+'"]',typeof a=="string"&&(r+='[crossorigin="'+a+'"]'),Qg.has(r)||(Qg.add(r),e={rel:e,crossOrigin:a,href:t},n.querySelector(r)===null&&(t=n.createElement("link"),yt(t,"link",e),ct(t),n.head.appendChild(t)))}}function OE(e){bn.D(e),R0("dns-prefetch",e,null)}function LE(e,t){bn.C(e,t),R0("preconnect",e,t)}function PE(e,t,a){bn.L(e,t,a);var n=Is;if(n&&e&&t){var r='link[rel="preload"][as="'+ga(t)+'"]';t==="image"&&a&&a.imageSrcSet?(r+='[imagesrcset="'+ga(a.imageSrcSet)+'"]',typeof a.imageSizes=="string"&&(r+='[imagesizes="'+ga(a.imageSizes)+'"]')):r+='[href="'+ga(e)+'"]';var s=r;switch(t){case"style":s=js(e);break;case"script":s=Ks(e)}$a.has(s)||(e=Ae({rel:"preload",href:t==="image"&&a&&a.imageSrcSet?void 0:e,as:t},a),$a.set(s,e),n.querySelector(r)!==null||t==="style"&&n.querySelector(jo(s))||t==="script"&&n.querySelector(Fo(s))||(t=n.createElement("link"),yt(t,"link",e),ct(t),n.head.appendChild(t)))}}function UE(e,t){bn.m(e,t);var a=Is;if(a&&e){var n=t&&typeof t.as=="string"?t.as:"script",r='link[rel="modulepreload"][as="'+ga(n)+'"][href="'+ga(e)+'"]',s=r;switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":s=Ks(e)}if(!$a.has(s)&&(e=Ae({rel:"modulepreload",href:e},t),$a.set(s,e),a.querySelector(r)===null)){switch(n){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(a.querySelector(Fo(s)))return}n=a.createElement("link"),yt(n,"link",e),ct(n),a.head.appendChild(n)}}}function jE(e,t,a){bn.S(e,t,a);var n=Is;if(n&&e){var r=xs(n).hoistableStyles,s=js(e);t=t||"default";var i=r.get(s);if(!i){var o={loading:0,preload:null};if(i=n.querySelector(jo(s)))o.loading=5;else{e=Ae({rel:"stylesheet",href:e,"data-precedence":t},a),(a=$a.get(s))&&tp(e,a);var u=i=n.createElement("link");ct(u),yt(u,"link",e),u._p=new Promise(function(c,d){u.onload=c,u.onerror=d}),u.addEventListener("load",function(){o.loading|=1}),u.addEventListener("error",function(){o.loading|=2}),o.loading|=4,cu(i,t,n)}i={type:"stylesheet",instance:i,count:1,state:o},r.set(s,i)}}}function FE(e,t){bn.X(e,t);var a=Is;if(a&&e){var n=xs(a).hoistableScripts,r=Ks(e),s=n.get(r);s||(s=a.querySelector(Fo(r)),s||(e=Ae({src:e,async:!0},t),(t=$a.get(r))&&ap(e,t),s=a.createElement("script"),ct(s),yt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function BE(e,t){bn.M(e,t);var a=Is;if(a&&e){var n=xs(a).hoistableScripts,r=Ks(e),s=n.get(r);s||(s=a.querySelector(Fo(r)),s||(e=Ae({src:e,async:!0,type:"module"},t),(t=$a.get(r))&&ap(e,t),s=a.createElement("script"),ct(s),yt(s,"link",e),a.head.appendChild(s)),s={type:"script",instance:s,count:1,state:null},n.set(r,s))}}function Vg(e,t,a,n){var r=(r=Qn.current)?ju(r):null;if(!r)throw Error(j(446));switch(e){case"meta":case"title":return null;case"style":return typeof a.precedence=="string"&&typeof a.href=="string"?(t=js(a.href),a=xs(r).hoistableStyles,n=a.get(t),n||(n={type:"style",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};case"link":if(a.rel==="stylesheet"&&typeof a.href=="string"&&typeof a.precedence=="string"){e=js(a.href);var s=xs(r).hoistableStyles,i=s.get(e);if(i||(r=r.ownerDocument||r,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},s.set(e,i),(s=r.querySelector(jo(e)))&&!s._p&&(i.instance=s,i.state.loading=5),$a.has(e)||(a={rel:"preload",as:"style",href:a.href,crossOrigin:a.crossOrigin,integrity:a.integrity,media:a.media,hrefLang:a.hrefLang,referrerPolicy:a.referrerPolicy},$a.set(e,a),s||zE(r,e,a,i.state))),t&&n===null)throw Error(j(528,""));return i}if(t&&n!==null)throw Error(j(529,""));return null;case"script":return t=a.async,a=a.src,typeof a=="string"&&t&&typeof t!="function"&&typeof t!="symbol"?(t=Ks(a),a=xs(r).hoistableScripts,n=a.get(t),n||(n={type:"script",instance:null,count:0,state:null},a.set(t,n)),n):{type:"void",instance:null,count:0,state:null};default:throw Error(j(444,e))}}function js(e){return'href="'+ga(e)+'"'}function jo(e){return'link[rel="stylesheet"]['+e+"]"}function C0(e){return Ae({},e,{"data-precedence":e.precedence,precedence:null})}function zE(e,t,a,n){e.querySelector('link[rel="preload"][as="style"]['+t+"]")?n.loading=1:(t=e.createElement("link"),n.preload=t,t.addEventListener("load",function(){return n.loading|=1}),t.addEventListener("error",function(){return n.loading|=2}),yt(t,"link",a),ct(t),e.head.appendChild(t))}function Ks(e){return'[src="'+ga(e)+'"]'}function Fo(e){return"script[async]"+e}function Gg(e,t,a){if(t.count++,t.instance===null)switch(t.type){case"style":var n=e.querySelector('style[data-href~="'+ga(a.href)+'"]');if(n)return t.instance=n,ct(n),n;var r=Ae({},a,{"data-href":a.href,"data-precedence":a.precedence,href:null,precedence:null});return n=(e.ownerDocument||e).createElement("style"),ct(n),yt(n,"style",r),cu(n,a.precedence,e),t.instance=n;case"stylesheet":r=js(a.href);var s=e.querySelector(jo(r));if(s)return t.state.loading|=4,t.instance=s,ct(s),s;n=C0(a),(r=$a.get(r))&&tp(n,r),s=(e.ownerDocument||e).createElement("link"),ct(s);var i=s;return i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),yt(s,"link",n),t.state.loading|=4,cu(s,a.precedence,e),t.instance=s;case"script":return s=Ks(a.src),(r=e.querySelector(Fo(s)))?(t.instance=r,ct(r),r):(n=a,(r=$a.get(s))&&(n=Ae({},a),ap(n,r)),e=e.ownerDocument||e,r=e.createElement("script"),ct(r),yt(r,"link",n),e.head.appendChild(r),t.instance=r);case"void":return null;default:throw Error(j(443,t.type))}else t.type==="stylesheet"&&(t.state.loading&4)===0&&(n=t.instance,t.state.loading|=4,cu(n,a.precedence,e));return t.instance}function cu(e,t,a){for(var n=a.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),r=n.length?n[n.length-1]:null,s=r,i=0;i<n.length;i++){var o=n[i];if(o.dataset.precedence===t)s=o;else if(s!==r)break}s?s.parentNode.insertBefore(e,s.nextSibling):(t=a.nodeType===9?a.head:a,t.insertBefore(e,t.firstChild))}function tp(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.title==null&&(e.title=t.title)}function ap(e,t){e.crossOrigin==null&&(e.crossOrigin=t.crossOrigin),e.referrerPolicy==null&&(e.referrerPolicy=t.referrerPolicy),e.integrity==null&&(e.integrity=t.integrity)}var du=null;function Yg(e,t,a){if(du===null){var n=new Map,r=du=new Map;r.set(a,n)}else r=du,n=r.get(a),n||(n=new Map,r.set(a,n));if(n.has(e))return n;for(n.set(e,null),a=a.getElementsByTagName(e),r=0;r<a.length;r++){var s=a[r];if(!(s[Co]||s[Nt]||e==="link"&&s.getAttribute("rel")==="stylesheet")&&s.namespaceURI!=="http://www.w3.org/2000/svg"){var i=s.getAttribute(t)||"";i=e+i;var o=n.get(i);o?o.push(s):n.set(i,[s])}}return n}function Jg(e,t,a){e=e.ownerDocument||e,e.head.insertBefore(a,t==="title"?e.querySelector("head > title"):null)}function qE(e,t,a){if(a===1||t.itemProp!=null)return!1;switch(e){case"meta":case"title":return!0;case"style":if(typeof t.precedence!="string"||typeof t.href!="string"||t.href==="")break;return!0;case"link":if(typeof t.rel!="string"||typeof t.href!="string"||t.href===""||t.onLoad||t.onError)break;switch(t.rel){case"stylesheet":return e=t.disabled,typeof t.precedence=="string"&&e==null;default:return!0}case"script":if(t.async&&typeof t.async!="function"&&typeof t.async!="symbol"&&!t.onLoad&&!t.onError&&t.src&&typeof t.src=="string")return!0}return!1}function E0(e){return!(e.type==="stylesheet"&&(e.state.loading&3)===0)}var bo=null;function IE(){}function KE(e,t,a){if(bo===null)throw Error(j(475));var n=bo;if(t.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&(t.state.loading&4)===0){if(t.instance===null){var r=js(a.href),s=e.querySelector(jo(r));if(s){e=s._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(n.count++,n=Fu.bind(n),e.then(n,n)),t.state.loading|=4,t.instance=s,ct(s);return}s=e.ownerDocument||e,a=C0(a),(r=$a.get(r))&&tp(a,r),s=s.createElement("link"),ct(s);var i=s;i._p=new Promise(function(o,u){i.onload=o,i.onerror=u}),yt(s,"link",a),t.instance=s}n.stylesheets===null&&(n.stylesheets=new Map),n.stylesheets.set(t,e),(e=t.state.preload)&&(t.state.loading&3)===0&&(n.count++,t=Fu.bind(n),e.addEventListener("load",t),e.addEventListener("error",t))}}function HE(){if(bo===null)throw Error(j(475));var e=bo;return e.stylesheets&&e.count===0&&cf(e,e.stylesheets),0<e.count?function(t){var a=setTimeout(function(){if(e.stylesheets&&cf(e,e.stylesheets),e.unsuspend){var n=e.unsuspend;e.unsuspend=null,n()}},6e4);return e.unsuspend=t,function(){e.unsuspend=null,clearTimeout(a)}}:null}function Fu(){if(this.count--,this.count===0){if(this.stylesheets)cf(this,this.stylesheets);else if(this.unsuspend){var e=this.unsuspend;this.unsuspend=null,e()}}}var Bu=null;function cf(e,t){e.stylesheets=null,e.unsuspend!==null&&(e.count++,Bu=new Map,t.forEach(QE,e),Bu=null,Fu.call(e))}function QE(e,t){if(!(t.state.loading&4)){var a=Bu.get(e);if(a)var n=a.get(null);else{a=new Map,Bu.set(e,a);for(var r=e.querySelectorAll("link[data-precedence],style[data-precedence]"),s=0;s<r.length;s++){var i=r[s];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(a.set(i.dataset.precedence,i),n=i)}n&&a.set(null,n)}r=t.instance,i=r.getAttribute("data-precedence"),s=a.get(i)||n,s===n&&a.set(null,r),a.set(i,r),this.count++,n=Fu.bind(this),r.addEventListener("load",n),r.addEventListener("error",n),s?s.parentNode.insertBefore(r,s.nextSibling):(e=e.nodeType===9?e.head:e,e.insertBefore(r,e.firstChild)),t.state.loading|=4}}var xo={$$typeof:ln,Provider:null,Consumer:null,_currentValue:xr,_currentValue2:xr,_threadCount:0};function VE(e,t,a,n,r,s,i,o){this.tag=1,this.containerInfo=e,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=zd(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=zd(0),this.hiddenUpdates=zd(null),this.identifierPrefix=n,this.onUncaughtError=r,this.onCaughtError=s,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=o,this.incompleteTransitions=new Map}function T0(e,t,a,n,r,s,i,o,u,c,d,f){return e=new VE(e,t,a,i,o,u,c,f),t=1,s===!0&&(t|=24),s=Xt(3,null,null,t),e.current=s,s.stateNode=e,t=Tf(),t.refCount++,e.pooledCache=t,t.refCount++,s.memoizedState={element:n,isDehydrated:a,cache:t},Df(s),e}function A0(e){return e?(e=vs,e):vs}function D0(e,t,a,n,r,s){r=A0(r),n.context===null?n.context=r:n.pendingContext=r,n=Vn(t),n.payload={element:a},s=s===void 0?null:s,s!==null&&(n.callback=s),a=Gn(e,n,t),a!==null&&(ta(a,e,t),eo(a,e,t))}function Xg(e,t){if(e=e.memoizedState,e!==null&&e.dehydrated!==null){var a=e.retryLane;e.retryLane=a!==0&&a<t?a:t}}function np(e,t){Xg(e,t),(e=e.alternate)&&Xg(e,t)}function M0(e){if(e.tag===13){var t=zs(e,67108864);t!==null&&ta(t,e,67108864),np(e,67108864)}}var zu=!0;function GE(e,t,a,n){var r=ne.T;ne.T=null;var s=ge.p;try{ge.p=2,rp(e,t,a,n)}finally{ge.p=s,ne.T=r}}function YE(e,t,a,n){var r=ne.T;ne.T=null;var s=ge.p;try{ge.p=8,rp(e,t,a,n)}finally{ge.p=s,ne.T=r}}function rp(e,t,a,n){if(zu){var r=df(n);if(r===null)hm(e,t,n,qu,a),Zg(e,n);else if(XE(r,e,t,a,n))n.stopPropagation();else if(Zg(e,n),t&4&&-1<JE.indexOf(e)){for(;r!==null;){var s=Bs(r);if(s!==null)switch(s.tag){case 3:if(s=s.stateNode,s.current.memoizedState.isDehydrated){var i=gr(s.pendingLanes);if(i!==0){var o=s;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var u=1<<31-Wt(i);o.entanglements[1]|=u,i&=~u}Ka(s),(we&6)===0&&(Du=za()+500,Uo(0,!1))}}break;case 13:o=zs(s,2),o!==null&&ta(o,s,2),Wu(),np(s,2)}if(s=df(n),s===null&&hm(e,t,n,qu,a),s===r)break;r=s}r!==null&&n.stopPropagation()}else hm(e,t,n,null,a)}}function df(e){return e=xf(e),sp(e)}var qu=null;function sp(e){if(qu=null,e=cs(e),e!==null){var t=No(e);if(t===null)e=null;else{var a=t.tag;if(a===13){if(e=ry(t),e!==null)return e;e=null}else if(a===3){if(t.stateNode.current.memoizedState.isDehydrated)return t.tag===3?t.stateNode.containerInfo:null;e=null}else t!==e&&(e=null)}}return qu=e,null}function O0(e){switch(e){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(UR()){case ly:return 2;case uy:return 8;case vu:case jR:return 32;case cy:return 268435456;default:return 32}default:return 32}}var mf=!1,Xn=null,Zn=null,Wn=null,$o=new Map,wo=new Map,zn=[],JE="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function Zg(e,t){switch(e){case"focusin":case"focusout":Xn=null;break;case"dragenter":case"dragleave":Zn=null;break;case"mouseover":case"mouseout":Wn=null;break;case"pointerover":case"pointerout":$o.delete(t.pointerId);break;case"gotpointercapture":case"lostpointercapture":wo.delete(t.pointerId)}}function Ii(e,t,a,n,r,s){return e===null||e.nativeEvent!==s?(e={blockedOn:t,domEventName:a,eventSystemFlags:n,nativeEvent:s,targetContainers:[r]},t!==null&&(t=Bs(t),t!==null&&M0(t)),e):(e.eventSystemFlags|=n,t=e.targetContainers,r!==null&&t.indexOf(r)===-1&&t.push(r),e)}function XE(e,t,a,n,r){switch(t){case"focusin":return Xn=Ii(Xn,e,t,a,n,r),!0;case"dragenter":return Zn=Ii(Zn,e,t,a,n,r),!0;case"mouseover":return Wn=Ii(Wn,e,t,a,n,r),!0;case"pointerover":var s=r.pointerId;return $o.set(s,Ii($o.get(s)||null,e,t,a,n,r)),!0;case"gotpointercapture":return s=r.pointerId,wo.set(s,Ii(wo.get(s)||null,e,t,a,n,r)),!0}return!1}function L0(e){var t=cs(e.target);if(t!==null){var a=No(t);if(a!==null){if(t=a.tag,t===13){if(t=ry(a),t!==null){e.blockedOn=t,QR(e.priority,function(){if(a.tag===13){var n=ea();n=vf(n);var r=zs(a,n);r!==null&&ta(r,a,n),np(a,n)}});return}}else if(t===3&&a.stateNode.current.memoizedState.isDehydrated){e.blockedOn=a.tag===3?a.stateNode.containerInfo:null;return}}}e.blockedOn=null}function mu(e){if(e.blockedOn!==null)return!1;for(var t=e.targetContainers;0<t.length;){var a=df(e.nativeEvent);if(a===null){a=e.nativeEvent;var n=new a.constructor(a.type,a);Cm=n,a.target.dispatchEvent(n),Cm=null}else return t=Bs(a),t!==null&&M0(t),e.blockedOn=a,!1;t.shift()}return!0}function Wg(e,t,a){mu(e)&&a.delete(t)}function ZE(){mf=!1,Xn!==null&&mu(Xn)&&(Xn=null),Zn!==null&&mu(Zn)&&(Zn=null),Wn!==null&&mu(Wn)&&(Wn=null),$o.forEach(Wg),wo.forEach(Wg)}function Zl(e,t){e.blockedOn===t&&(e.blockedOn=null,mf||(mf=!0,st.unstable_scheduleCallback(st.unstable_NormalPriority,ZE)))}var Wl=null;function ey(e){Wl!==e&&(Wl=e,st.unstable_scheduleCallback(st.unstable_NormalPriority,function(){Wl===e&&(Wl=null);for(var t=0;t<e.length;t+=3){var a=e[t],n=e[t+1],r=e[t+2];if(typeof n!="function"){if(sp(n||a)===null)continue;break}var s=Bs(a);s!==null&&(e.splice(t,3),t-=3,Km(s,{pending:!0,data:r,method:a.method,action:n},n,r))}}))}function So(e){function t(u){return Zl(u,e)}Xn!==null&&Zl(Xn,e),Zn!==null&&Zl(Zn,e),Wn!==null&&Zl(Wn,e),$o.forEach(t),wo.forEach(t);for(var a=0;a<zn.length;a++){var n=zn[a];n.blockedOn===e&&(n.blockedOn=null)}for(;0<zn.length&&(a=zn[0],a.blockedOn===null);)L0(a),a.blockedOn===null&&zn.shift();if(a=(e.ownerDocument||e).$$reactFormReplay,a!=null)for(n=0;n<a.length;n+=3){var r=a[n],s=a[n+1],i=r[qt]||null;if(typeof s=="function")i||ey(a);else if(i){var o=null;if(s&&s.hasAttribute("formAction")){if(r=s,i=s[qt]||null)o=i.formAction;else if(sp(r)!==null)continue}else o=i.action;typeof o=="function"?a[n+1]=o:(a.splice(n,3),n-=3),ey(a)}}}function ip(e){this._internalRoot=e}nc.prototype.render=ip.prototype.render=function(e){var t=this._internalRoot;if(t===null)throw Error(j(409));var a=t.current,n=ea();D0(a,n,e,t,null,null)};nc.prototype.unmount=ip.prototype.unmount=function(){var e=this._internalRoot;if(e!==null){this._internalRoot=null;var t=e.containerInfo;D0(e.current,2,null,e,null,null),Wu(),t[Fs]=null}};function nc(e){this._internalRoot=e}nc.prototype.unstable_scheduleHydration=function(e){if(e){var t=hy();e={blockedOn:null,target:e,priority:t};for(var a=0;a<zn.length&&t!==0&&t<zn[a].priority;a++);zn.splice(a,0,e),a===0&&L0(e)}};var ty=ay.version;if(ty!=="19.1.0")throw Error(j(527,ty,"19.1.0"));ge.findDOMNode=function(e){var t=e._reactInternals;if(t===void 0)throw typeof e.render=="function"?Error(j(188)):(e=Object.keys(e).join(","),Error(j(268,e)));return e=TR(t),e=e!==null?sy(e):null,e=e===null?null:e.stateNode,e};var WE={bundleType:0,version:"19.1.0",rendererPackageName:"react-dom",currentDispatcherRef:ne,reconcilerVersion:"19.1.0"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(Ki=__REACT_DEVTOOLS_GLOBAL_HOOK__,!Ki.isDisabled&&Ki.supportsFiber))try{_o=Ki.inject(WE),Zt=Ki}catch{}var Ki;rc.createRoot=function(e,t){if(!ny(e))throw Error(j(299));var a=!1,n="",r=Tb,s=Ab,i=Db,o=null;return t!=null&&(t.unstable_strictMode===!0&&(a=!0),t.identifierPrefix!==void 0&&(n=t.identifierPrefix),t.onUncaughtError!==void 0&&(r=t.onUncaughtError),t.onCaughtError!==void 0&&(s=t.onCaughtError),t.onRecoverableError!==void 0&&(i=t.onRecoverableError),t.unstable_transitionCallbacks!==void 0&&(o=t.unstable_transitionCallbacks)),t=T0(e,1,!1,null,null,a,n,r,s,i,o,null),e[Fs]=t.current,ep(e),new ip(t)};rc.hydrateRoot=function(e,t,a){if(!ny(e))throw Error(j(299));var n=!1,r="",s=Tb,i=Ab,o=Db,u=null,c=null;return a!=null&&(a.unstable_strictMode===!0&&(n=!0),a.identifierPrefix!==void 0&&(r=a.identifierPrefix),a.onUncaughtError!==void 0&&(s=a.onUncaughtError),a.onCaughtError!==void 0&&(i=a.onCaughtError),a.onRecoverableError!==void 0&&(o=a.onRecoverableError),a.unstable_transitionCallbacks!==void 0&&(u=a.unstable_transitionCallbacks),a.formState!==void 0&&(c=a.formState)),t=T0(e,1,!0,t,a??null,n,r,s,i,o,u,c),t.context=A0(null),a=t.current,n=ea(),n=vf(n),r=Vn(n),r.callback=null,Gn(a,r,n),a=n,t.current.lanes=a,Ro(t,a),Ka(t),e[Fs]=t.current,ep(e),new nc(t)};rc.version="19.1.0"});var F0=En((U6,j0)=>{"use strict";function U0(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(U0)}catch(e){console.error(e)}}U0(),j0.exports=P0()});var Pt=class{constructor(){this.listeners=new Set,this.subscribe=this.subscribe.bind(this)}subscribe(e){return this.listeners.add(e),this.onSubscribe(),()=>{this.listeners.delete(e),this.onUnsubscribe()}}hasListeners(){return this.listeners.size>0}onSubscribe(){}onUnsubscribe(){}};var uR={setTimeout:(e,t)=>setTimeout(e,t),clearTimeout:e=>clearTimeout(e),setInterval:(e,t)=>setInterval(e,t),clearInterval:e=>clearInterval(e)},cR=class{#t=uR;#e=!1;setTimeoutProvider(e){this.#t=e}setTimeout(e,t){return this.#t.setTimeout(e,t)}clearTimeout(e){this.#t.clearTimeout(e)}setInterval(e,t){return this.#t.setInterval(e,t)}clearInterval(e){this.#t.clearInterval(e)}},Ma=new cR;function Gh(e){setTimeout(e,0)}var Ut=typeof window>"u"||"Deno"in globalThis;function De(){}function Xh(e,t){return typeof e=="function"?e(t):e}function _i(e){return typeof e=="number"&&e>=0&&e!==1/0}function vl(e,t){return Math.max(e+(t||0)-Date.now(),0)}function Na(e,t){return typeof e=="function"?e(t):e}function jt(e,t){return typeof e=="function"?e(t):e}function gl(e,t){let{type:a="all",exact:n,fetchStatus:r,predicate:s,queryKey:i,stale:o}=e;if(i){if(n){if(t.queryHash!==ki(i,t.options))return!1}else if(!pr(t.queryKey,i))return!1}if(a!=="all"){let u=t.isActive();if(a==="active"&&!u||a==="inactive"&&u)return!1}return!(typeof o=="boolean"&&t.isStale()!==o||r&&r!==t.state.fetchStatus||s&&!s(t))}function yl(e,t){let{exact:a,status:n,predicate:r,mutationKey:s}=e;if(s){if(!t.options.mutationKey)return!1;if(a){if(Oa(t.options.mutationKey)!==Oa(s))return!1}else if(!pr(t.options.mutationKey,s))return!1}return!(n&&t.state.status!==n||r&&!r(t))}function ki(e,t){return(t?.queryKeyHashFn||Oa)(e)}function Oa(e){return JSON.stringify(e,(t,a)=>yd(a)?Object.keys(a).sort().reduce((n,r)=>(n[r]=a[r],n),{}):a)}function pr(e,t){return e===t?!0:typeof e!=typeof t?!1:e&&t&&typeof e=="object"&&typeof t=="object"?Object.keys(t).every(a=>pr(e[a],t[a])):!1}var dR=Object.prototype.hasOwnProperty;function Ri(e,t){if(e===t)return e;let a=Yh(e)&&Yh(t);if(!a&&!(yd(e)&&yd(t)))return t;let r=(a?e:Object.keys(e)).length,s=a?t:Object.keys(t),i=s.length,o=a?new Array(i):{},u=0;for(let c=0;c<i;c++){let d=a?c:s[c],f=e[d],m=t[d];if(f===m){o[d]=f,(a?c<r:dR.call(e,d))&&u++;continue}if(f===null||m===null||typeof f!="object"||typeof m!="object"){o[d]=m;continue}let h=Ri(f,m);o[d]=h,h===f&&u++}return r===i&&u===r?e:o}function Tn(e,t){if(!t||Object.keys(e).length!==Object.keys(t).length)return!1;for(let a in e)if(e[a]!==t[a])return!1;return!0}function Yh(e){return Array.isArray(e)&&e.length===Object.keys(e).length}function yd(e){if(!Jh(e))return!1;let t=e.constructor;if(t===void 0)return!0;let a=t.prototype;return!(!Jh(a)||!a.hasOwnProperty("isPrototypeOf")||Object.getPrototypeOf(e)!==Object.prototype)}function Jh(e){return Object.prototype.toString.call(e)==="[object Object]"}function Zh(e){return new Promise(t=>{Ma.setTimeout(t,e)})}function Ci(e,t,a){return typeof a.structuralSharing=="function"?a.structuralSharing(e,t):a.structuralSharing!==!1?Ri(e,t):t}function Wh(e,t,a=0){let n=[...e,t];return a&&n.length>a?n.slice(1):n}function ev(e,t,a=0){let n=[t,...e];return a&&n.length>a?n.slice(0,-1):n}var Yr=Symbol();function bl(e,t){return!e.queryFn&&t?.initialPromise?()=>t.initialPromise:!e.queryFn||e.queryFn===Yr?()=>Promise.reject(new Error(`Missing queryFn: '${e.queryHash}'`)):e.queryFn}function Ei(e,t){return typeof e=="function"?e(...t):!!e}var mR=class extends Pt{#t;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e();return window.addEventListener("visibilitychange",t,!1),()=>{window.removeEventListener("visibilitychange",t)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(t=>{typeof t=="boolean"?this.setFocused(t):this.onFocus()})}setFocused(e){this.#t!==e&&(this.#t=e,this.onFocus())}onFocus(){let e=this.isFocused();this.listeners.forEach(t=>{t(e)})}isFocused(){return typeof this.#t=="boolean"?this.#t:globalThis.document?.visibilityState!=="hidden"}},Jr=new mR;function Ti(){let e,t,a=new Promise((r,s)=>{e=r,t=s});a.status="pending",a.catch(()=>{});function n(r){Object.assign(a,r),delete a.resolve,delete a.reject}return a.resolve=r=>{n({status:"fulfilled",value:r}),e(r)},a.reject=r=>{n({status:"rejected",reason:r}),t(r)},a}var tv=Gh;function fR(){let e=[],t=0,a=o=>{o()},n=o=>{o()},r=tv,s=o=>{t?e.push(o):r(()=>{a(o)})},i=()=>{let o=e;e=[],o.length&&r(()=>{n(()=>{o.forEach(u=>{a(u)})})})};return{batch:o=>{let u;t++;try{u=o()}finally{t--,t||i()}return u},batchCalls:o=>(...u)=>{s(()=>{o(...u)})},schedule:s,setNotifyFunction:o=>{a=o},setBatchNotifyFunction:o=>{n=o},setScheduler:o=>{r=o}}}var ce=fR();var pR=class extends Pt{#t=!0;#e;#a;constructor(){super(),this.#a=e=>{if(!Ut&&window.addEventListener){let t=()=>e(!0),a=()=>e(!1);return window.addEventListener("online",t,!1),window.addEventListener("offline",a,!1),()=>{window.removeEventListener("online",t),window.removeEventListener("offline",a)}}}}onSubscribe(){this.#e||this.setEventListener(this.#a)}onUnsubscribe(){this.hasListeners()||(this.#e?.(),this.#e=void 0)}setEventListener(e){this.#a=e,this.#e?.(),this.#e=e(this.setOnline.bind(this))}setOnline(e){this.#t!==e&&(this.#t=e,this.listeners.forEach(a=>{a(e)}))}isOnline(){return this.#t}},Xr=new pR;function hR(e){return Math.min(1e3*2**e,3e4)}function bd(e){return(e??"online")==="online"?Xr.isOnline():!0}var xl=class extends Error{constructor(e){super("CancelledError"),this.revert=e?.revert,this.silent=e?.silent}};function $l(e){let t=!1,a=0,n,r=Ti(),s=()=>r.status!=="pending",i=y=>{if(!s()){let w=new xl(y);m(w),e.onCancel?.(w)}},o=()=>{t=!0},u=()=>{t=!1},c=()=>Jr.isFocused()&&(e.networkMode==="always"||Xr.isOnline())&&e.canRun(),d=()=>bd(e.networkMode)&&e.canRun(),f=y=>{s()||(n?.(),r.resolve(y))},m=y=>{s()||(n?.(),r.reject(y))},h=()=>new Promise(y=>{n=w=>{(s()||c())&&y(w)},e.onPause?.()}).then(()=>{n=void 0,s()||e.onContinue?.()}),b=()=>{if(s())return;let y,w=a===0?e.initialPromise:void 0;try{y=w??e.fn()}catch(g){y=Promise.reject(g)}Promise.resolve(y).then(f).catch(g=>{if(s())return;let v=e.retry??(Ut?0:3),x=e.retryDelay??hR,$=typeof x=="function"?x(a,g):x,S=v===!0||typeof v=="number"&&a<v||typeof v=="function"&&v(a,g);if(t||!S){m(g);return}a++,e.onFail?.(a,g),Zh($).then(()=>c()?void 0:h()).then(()=>{t?m(g):b()})})};return{promise:r,status:()=>r.status,cancel:i,continue:()=>(n?.(),r),cancelRetry:o,continueRetry:u,canStart:d,start:()=>(d()?b():h().then(b),r)}}var wl=class{#t;destroy(){this.clearGcTimeout()}scheduleGc(){this.clearGcTimeout(),_i(this.gcTime)&&(this.#t=Ma.setTimeout(()=>{this.optionalRemove()},this.gcTime))}updateGcTime(e){this.gcTime=Math.max(this.gcTime||0,e??(Ut?1/0:5*60*1e3))}clearGcTimeout(){this.#t&&(Ma.clearTimeout(this.#t),this.#t=void 0)}};var nv=class extends wl{#t;#e;#a;#n;#r;#s;#o;constructor(e){super(),this.#o=!1,this.#s=e.defaultOptions,this.setOptions(e.options),this.observers=[],this.#n=e.client,this.#a=this.#n.getQueryCache(),this.queryKey=e.queryKey,this.queryHash=e.queryHash,this.#t=av(this.options),this.state=e.state??this.#t,this.scheduleGc()}get meta(){return this.options.meta}get promise(){return this.#r?.promise}setOptions(e){if(this.options={...this.#s,...e},this.updateGcTime(this.options.gcTime),this.state&&this.state.data===void 0){let t=av(this.options);t.data!==void 0&&(this.setData(t.data,{updatedAt:t.dataUpdatedAt,manual:!0}),this.#t=t)}}optionalRemove(){!this.observers.length&&this.state.fetchStatus==="idle"&&this.#a.remove(this)}setData(e,t){let a=Ci(this.state.data,e,this.options);return this.#i({data:a,type:"success",dataUpdatedAt:t?.updatedAt,manual:t?.manual}),a}setState(e,t){this.#i({type:"setState",state:e,setStateOptions:t})}cancel(e){let t=this.#r?.promise;return this.#r?.cancel(e),t?t.then(De).catch(De):Promise.resolve()}destroy(){super.destroy(),this.cancel({silent:!0})}reset(){this.destroy(),this.setState(this.#t)}isActive(){return this.observers.some(e=>jt(e.options.enabled,this)!==!1)}isDisabled(){return this.getObserversCount()>0?!this.isActive():this.options.queryFn===Yr||this.state.dataUpdateCount+this.state.errorUpdateCount===0}isStatic(){return this.getObserversCount()>0?this.observers.some(e=>Na(e.options.staleTime,this)==="static"):!1}isStale(){return this.getObserversCount()>0?this.observers.some(e=>e.getCurrentResult().isStale):this.state.data===void 0||this.state.isInvalidated}isStaleByTime(e=0){return this.state.data===void 0?!0:e==="static"?!1:this.state.isInvalidated?!0:!vl(this.state.dataUpdatedAt,e)}onFocus(){this.observers.find(t=>t.shouldFetchOnWindowFocus())?.refetch({cancelRefetch:!1}),this.#r?.continue()}onOnline(){this.observers.find(t=>t.shouldFetchOnReconnect())?.refetch({cancelRefetch:!1}),this.#r?.continue()}addObserver(e){this.observers.includes(e)||(this.observers.push(e),this.clearGcTimeout(),this.#a.notify({type:"observerAdded",query:this,observer:e}))}removeObserver(e){this.observers.includes(e)&&(this.observers=this.observers.filter(t=>t!==e),this.observers.length||(this.#r&&(this.#o?this.#r.cancel({revert:!0}):this.#r.cancelRetry()),this.scheduleGc()),this.#a.notify({type:"observerRemoved",query:this,observer:e}))}getObserversCount(){return this.observers.length}invalidate(){this.state.isInvalidated||this.#i({type:"invalidate"})}async fetch(e,t){if(this.state.fetchStatus!=="idle"&&this.#r?.status()!=="rejected"){if(this.state.data!==void 0&&t?.cancelRefetch)this.cancel({silent:!0});else if(this.#r)return this.#r.continueRetry(),this.#r.promise}if(e&&this.setOptions(e),!this.options.queryFn){let o=this.observers.find(u=>u.options.queryFn);o&&this.setOptions(o.options)}let a=new AbortController,n=o=>{Object.defineProperty(o,"signal",{enumerable:!0,get:()=>(this.#o=!0,a.signal)})},r=()=>{let o=bl(this.options,t),c=(()=>{let d={client:this.#n,queryKey:this.queryKey,meta:this.meta};return n(d),d})();return this.#o=!1,this.options.persister?this.options.persister(o,c,this):o(c)},i=(()=>{let o={fetchOptions:t,options:this.options,queryKey:this.queryKey,client:this.#n,state:this.state,fetchFn:r};return n(o),o})();this.options.behavior?.onFetch(i,this),this.#e=this.state,(this.state.fetchStatus==="idle"||this.state.fetchMeta!==i.fetchOptions?.meta)&&this.#i({type:"fetch",meta:i.fetchOptions?.meta}),this.#r=$l({initialPromise:t?.initialPromise,fn:i.fetchFn,onCancel:o=>{o instanceof xl&&o.revert&&this.setState({...this.#e,fetchStatus:"idle"}),a.abort()},onFail:(o,u)=>{this.#i({type:"failed",failureCount:o,error:u})},onPause:()=>{this.#i({type:"pause"})},onContinue:()=>{this.#i({type:"continue"})},retry:i.options.retry,retryDelay:i.options.retryDelay,networkMode:i.options.networkMode,canRun:()=>!0});try{let o=await this.#r.start();if(o===void 0)throw new Error(`${this.queryHash} data is undefined`);return this.setData(o),this.#a.config.onSuccess?.(o,this),this.#a.config.onSettled?.(o,this.state.error,this),o}catch(o){if(o instanceof xl){if(o.silent)return this.#r.promise;if(o.revert){if(this.state.data===void 0)throw o;return this.state.data}}throw this.#i({type:"error",error:o}),this.#a.config.onError?.(o,this),this.#a.config.onSettled?.(this.state.data,o,this),o}finally{this.scheduleGc()}}#i(e){let t=a=>{switch(e.type){case"failed":return{...a,fetchFailureCount:e.failureCount,fetchFailureReason:e.error};case"pause":return{...a,fetchStatus:"paused"};case"continue":return{...a,fetchStatus:"fetching"};case"fetch":return{...a,...xd(a.data,this.options),fetchMeta:e.meta??null};case"success":let n={...a,data:e.data,dataUpdateCount:a.dataUpdateCount+1,dataUpdatedAt:e.dataUpdatedAt??Date.now(),error:null,isInvalidated:!1,status:"success",...!e.manual&&{fetchStatus:"idle",fetchFailureCount:0,fetchFailureReason:null}};return this.#e=e.manual?n:void 0,n;case"error":let r=e.error;return{...a,error:r,errorUpdateCount:a.errorUpdateCount+1,errorUpdatedAt:Date.now(),fetchFailureCount:a.fetchFailureCount+1,fetchFailureReason:r,fetchStatus:"idle",status:"error"};case"invalidate":return{...a,isInvalidated:!0};case"setState":return{...a,...e.state}}};this.state=t(this.state),ce.batch(()=>{this.observers.forEach(a=>{a.onQueryUpdate()}),this.#a.notify({query:this,type:"updated",action:e})})}};function xd(e,t){return{fetchFailureCount:0,fetchFailureReason:null,fetchStatus:bd(t.networkMode)?"fetching":"paused",...e===void 0&&{error:null,status:"pending"}}}function av(e){let t=typeof e.initialData=="function"?e.initialData():e.initialData,a=t!==void 0,n=a?typeof e.initialDataUpdatedAt=="function"?e.initialDataUpdatedAt():e.initialDataUpdatedAt:0;return{data:t,dataUpdateCount:0,dataUpdatedAt:a?n??Date.now():0,error:null,errorUpdateCount:0,errorUpdatedAt:0,fetchFailureCount:0,fetchFailureReason:null,fetchMeta:null,isInvalidated:!1,status:a?"success":"pending",fetchStatus:"idle"}}var hr=class extends Pt{constructor(e,t){super(),this.options=t,this.#t=e,this.#i=null,this.#o=Ti(),this.bindMethods(),this.setOptions(t)}#t;#e=void 0;#a=void 0;#n=void 0;#r;#s;#o;#i;#f;#d;#m;#u;#c;#l;#h=new Set;bindMethods(){this.refetch=this.refetch.bind(this)}onSubscribe(){this.listeners.size===1&&(this.#e.addObserver(this),rv(this.#e,this.options)?this.#p():this.updateResult(),this.#b())}onUnsubscribe(){this.hasListeners()||this.destroy()}shouldFetchOnReconnect(){return $d(this.#e,this.options,this.options.refetchOnReconnect)}shouldFetchOnWindowFocus(){return $d(this.#e,this.options,this.options.refetchOnWindowFocus)}destroy(){this.listeners=new Set,this.#x(),this.#$(),this.#e.removeObserver(this)}setOptions(e){let t=this.options,a=this.#e;if(this.options=this.#t.defaultQueryOptions(e),this.options.enabled!==void 0&&typeof this.options.enabled!="boolean"&&typeof this.options.enabled!="function"&&typeof jt(this.options.enabled,this.#e)!="boolean")throw new Error("Expected enabled to be a boolean or a callback that returns a boolean");this.#w(),this.#e.setOptions(this.options),t._defaulted&&!Tn(this.options,t)&&this.#t.getQueryCache().notify({type:"observerOptionsUpdated",query:this.#e,observer:this});let n=this.hasListeners();n&&sv(this.#e,a,this.options,t)&&this.#p(),this.updateResult(),n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||Na(this.options.staleTime,this.#e)!==Na(t.staleTime,this.#e))&&this.#v();let r=this.#g();n&&(this.#e!==a||jt(this.options.enabled,this.#e)!==jt(t.enabled,this.#e)||r!==this.#l)&&this.#y(r)}getOptimisticResult(e){let t=this.#t.getQueryCache().build(this.#t,e),a=this.createResult(t,e);return gR(this,a)&&(this.#n=a,this.#s=this.options,this.#r=this.#e.state),a}getCurrentResult(){return this.#n}trackResult(e,t){return new Proxy(e,{get:(a,n)=>(this.trackProp(n),t?.(n),n==="promise"&&!this.options.experimental_prefetchInRender&&this.#o.status==="pending"&&this.#o.reject(new Error("experimental_prefetchInRender feature flag is not enabled")),Reflect.get(a,n))})}trackProp(e){this.#h.add(e)}getCurrentQuery(){return this.#e}refetch({...e}={}){return this.fetch({...e})}fetchOptimistic(e){let t=this.#t.defaultQueryOptions(e),a=this.#t.getQueryCache().build(this.#t,t);return a.fetch().then(()=>this.createResult(a,t))}fetch(e){return this.#p({...e,cancelRefetch:e.cancelRefetch??!0}).then(()=>(this.updateResult(),this.#n))}#p(e){this.#w();let t=this.#e.fetch(this.options,e);return e?.throwOnError||(t=t.catch(De)),t}#v(){this.#x();let e=Na(this.options.staleTime,this.#e);if(Ut||this.#n.isStale||!_i(e))return;let a=vl(this.#n.dataUpdatedAt,e)+1;this.#u=Ma.setTimeout(()=>{this.#n.isStale||this.updateResult()},a)}#g(){return(typeof this.options.refetchInterval=="function"?this.options.refetchInterval(this.#e):this.options.refetchInterval)??!1}#y(e){this.#$(),this.#l=e,!(Ut||jt(this.options.enabled,this.#e)===!1||!_i(this.#l)||this.#l===0)&&(this.#c=Ma.setInterval(()=>{(this.options.refetchIntervalInBackground||Jr.isFocused())&&this.#p()},this.#l))}#b(){this.#v(),this.#y(this.#g())}#x(){this.#u&&(Ma.clearTimeout(this.#u),this.#u=void 0)}#$(){this.#c&&(Ma.clearInterval(this.#c),this.#c=void 0)}createResult(e,t){let a=this.#e,n=this.options,r=this.#n,s=this.#r,i=this.#s,u=e!==a?e.state:this.#a,{state:c}=e,d={...c},f=!1,m;if(t._optimisticResults){let T=this.hasListeners(),M=!T&&rv(e,t),O=T&&sv(e,a,t,n);(M||O)&&(d={...d,...xd(c.data,e.options)}),t._optimisticResults==="isRestoring"&&(d.fetchStatus="idle")}let{error:h,errorUpdatedAt:b,status:y}=d;m=d.data;let w=!1;if(t.placeholderData!==void 0&&m===void 0&&y==="pending"){let T;r?.isPlaceholderData&&t.placeholderData===i?.placeholderData?(T=r.data,w=!0):T=typeof t.placeholderData=="function"?t.placeholderData(this.#m?.state.data,this.#m):t.placeholderData,T!==void 0&&(y="success",m=Ci(r?.data,T,t),f=!0)}if(t.select&&m!==void 0&&!w)if(r&&m===s?.data&&t.select===this.#f)m=this.#d;else try{this.#f=t.select,m=t.select(m),m=Ci(r?.data,m,t),this.#d=m,this.#i=null}catch(T){this.#i=T}this.#i&&(h=this.#i,m=this.#d,b=Date.now(),y="error");let g=d.fetchStatus==="fetching",v=y==="pending",x=y==="error",$=v&&g,S=m!==void 0,_={status:y,fetchStatus:d.fetchStatus,isPending:v,isSuccess:y==="success",isError:x,isInitialLoading:$,isLoading:$,data:m,dataUpdatedAt:d.dataUpdatedAt,error:h,errorUpdatedAt:b,failureCount:d.fetchFailureCount,failureReason:d.fetchFailureReason,errorUpdateCount:d.errorUpdateCount,isFetched:d.dataUpdateCount>0||d.errorUpdateCount>0,isFetchedAfterMount:d.dataUpdateCount>u.dataUpdateCount||d.errorUpdateCount>u.errorUpdateCount,isFetching:g,isRefetching:g&&!v,isLoadingError:x&&!S,isPaused:d.fetchStatus==="paused",isPlaceholderData:f,isRefetchError:x&&S,isStale:wd(e,t),refetch:this.refetch,promise:this.#o,isEnabled:jt(t.enabled,e)!==!1};if(this.options.experimental_prefetchInRender){let T=U=>{_.status==="error"?U.reject(_.error):_.data!==void 0&&U.resolve(_.data)},M=()=>{let U=this.#o=_.promise=Ti();T(U)},O=this.#o;switch(O.status){case"pending":e.queryHash===a.queryHash&&T(O);break;case"fulfilled":(_.status==="error"||_.data!==O.value)&&M();break;case"rejected":(_.status!=="error"||_.error!==O.reason)&&M();break}}return _}updateResult(){let e=this.#n,t=this.createResult(this.#e,this.options);if(this.#r=this.#e.state,this.#s=this.options,this.#r.data!==void 0&&(this.#m=this.#e),Tn(t,e))return;this.#n=t;let a=()=>{if(!e)return!0;let{notifyOnChangeProps:n}=this.options,r=typeof n=="function"?n():n;if(r==="all"||!r&&!this.#h.size)return!0;let s=new Set(r??this.#h);return this.options.throwOnError&&s.add("error"),Object.keys(this.#n).some(i=>{let o=i;return this.#n[o]!==e[o]&&s.has(o)})};this.#S({listeners:a()})}#w(){let e=this.#t.getQueryCache().build(this.#t,this.options);if(e===this.#e)return;let t=this.#e;this.#e=e,this.#a=e.state,this.hasListeners()&&(t?.removeObserver(this),e.addObserver(this))}onQueryUpdate(){this.updateResult(),this.hasListeners()&&this.#b()}#S(e){ce.batch(()=>{e.listeners&&this.listeners.forEach(t=>{t(this.#n)}),this.#t.getQueryCache().notify({query:this.#e,type:"observerResultsUpdated"})})}};function vR(e,t){return jt(t.enabled,e)!==!1&&e.state.data===void 0&&!(e.state.status==="error"&&t.retryOnMount===!1)}function rv(e,t){return vR(e,t)||e.state.data!==void 0&&$d(e,t,t.refetchOnMount)}function $d(e,t,a){if(jt(t.enabled,e)!==!1&&Na(t.staleTime,e)!=="static"){let n=typeof a=="function"?a(e):a;return n==="always"||n!==!1&&wd(e,t)}return!1}function sv(e,t,a,n){return(e!==t||jt(n.enabled,e)===!1)&&(!a.suspense||e.state.status!=="error")&&wd(e,a)}function wd(e,t){return jt(t.enabled,e)!==!1&&e.isStaleByTime(Na(t.staleTime,e))}function gR(e,t){return!Tn(e.getCurrentResult(),t)}function Sd(e){return{onFetch:(t,a)=>{let n=t.options,r=t.fetchOptions?.meta?.fetchMore?.direction,s=t.state.data?.pages||[],i=t.state.data?.pageParams||[],o={pages:[],pageParams:[]},u=0,c=async()=>{let d=!1,f=b=>{Object.defineProperty(b,"signal",{enumerable:!0,get:()=>(t.signal.aborted?d=!0:t.signal.addEventListener("abort",()=>{d=!0}),t.signal)})},m=bl(t.options,t.fetchOptions),h=async(b,y,w)=>{if(d)return Promise.reject();if(y==null&&b.pages.length)return Promise.resolve(b);let v=(()=>{let C={client:t.client,queryKey:t.queryKey,pageParam:y,direction:w?"backward":"forward",meta:t.options.meta};return f(C),C})(),x=await m(v),{maxPages:$}=t.options,S=w?ev:Wh;return{pages:S(b.pages,x,$),pageParams:S(b.pageParams,y,$)}};if(r&&s.length){let b=r==="backward",y=b?yR:iv,w={pages:s,pageParams:i},g=y(n,w);o=await h(w,g,b)}else{let b=e??s.length;do{let y=u===0?i[0]??n.initialPageParam:iv(n,o);if(u>0&&y==null)break;o=await h(o,y),u++}while(u<b)}return o};t.options.persister?t.fetchFn=()=>t.options.persister?.(c,{client:t.client,queryKey:t.queryKey,meta:t.options.meta,signal:t.signal},a):t.fetchFn=c}}}function iv(e,{pages:t,pageParams:a}){let n=t.length-1;return t.length>0?e.getNextPageParam(t[n],t,a[n],a):void 0}function yR(e,{pages:t,pageParams:a}){return t.length>0?e.getPreviousPageParam?.(t[0],t,a[0],a):void 0}var ov=class extends wl{#t;#e;#a;constructor(e){super(),this.mutationId=e.mutationId,this.#e=e.mutationCache,this.#t=[],this.state=e.state||Nd(),this.setOptions(e.options),this.scheduleGc()}setOptions(e){this.options=e,this.updateGcTime(this.options.gcTime)}get meta(){return this.options.meta}addObserver(e){this.#t.includes(e)||(this.#t.push(e),this.clearGcTimeout(),this.#e.notify({type:"observerAdded",mutation:this,observer:e}))}removeObserver(e){this.#t=this.#t.filter(t=>t!==e),this.scheduleGc(),this.#e.notify({type:"observerRemoved",mutation:this,observer:e})}optionalRemove(){this.#t.length||(this.state.status==="pending"?this.scheduleGc():this.#e.remove(this))}continue(){return this.#a?.continue()??this.execute(this.state.variables)}async execute(e){let t=()=>{this.#n({type:"continue"})};this.#a=$l({fn:()=>this.options.mutationFn?this.options.mutationFn(e):Promise.reject(new Error("No mutationFn found")),onFail:(r,s)=>{this.#n({type:"failed",failureCount:r,error:s})},onPause:()=>{this.#n({type:"pause"})},onContinue:t,retry:this.options.retry??0,retryDelay:this.options.retryDelay,networkMode:this.options.networkMode,canRun:()=>this.#e.canRun(this)});let a=this.state.status==="pending",n=!this.#a.canStart();try{if(a)t();else{this.#n({type:"pending",variables:e,isPaused:n}),await this.#e.config.onMutate?.(e,this);let s=await this.options.onMutate?.(e);s!==this.state.context&&this.#n({type:"pending",context:s,variables:e,isPaused:n})}let r=await this.#a.start();return await this.#e.config.onSuccess?.(r,e,this.state.context,this),await this.options.onSuccess?.(r,e,this.state.context),await this.#e.config.onSettled?.(r,null,this.state.variables,this.state.context,this),await this.options.onSettled?.(r,null,e,this.state.context),this.#n({type:"success",data:r}),r}catch(r){try{throw await this.#e.config.onError?.(r,e,this.state.context,this),await this.options.onError?.(r,e,this.state.context),await this.#e.config.onSettled?.(void 0,r,this.state.variables,this.state.context,this),await this.options.onSettled?.(void 0,r,e,this.state.context),r}finally{this.#n({type:"error",error:r})}}finally{this.#e.runNext(this)}}#n(e){let t=a=>{switch(e.type){case"failed":return{...a,failureCount:e.failureCount,failureReason:e.error};case"pause":return{...a,isPaused:!0};case"continue":return{...a,isPaused:!1};case"pending":return{...a,context:e.context,data:void 0,failureCount:0,failureReason:null,error:null,isPaused:e.isPaused,status:"pending",variables:e.variables,submittedAt:Date.now()};case"success":return{...a,data:e.data,failureCount:0,failureReason:null,error:null,status:"success",isPaused:!1};case"error":return{...a,data:void 0,error:e.error,failureCount:a.failureCount+1,failureReason:e.error,isPaused:!1,status:"error"}}};this.state=t(this.state),ce.batch(()=>{this.#t.forEach(a=>{a.onMutationUpdate(e)}),this.#e.notify({mutation:this,type:"updated",action:e})})}};function Nd(){return{context:void 0,data:void 0,error:null,failureCount:0,failureReason:null,isPaused:!1,status:"idle",variables:void 0,submittedAt:0}}var lv=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Set,this.#e=new Map,this.#a=0}#t;#e;#a;build(e,t,a){let n=new ov({mutationCache:this,mutationId:++this.#a,options:e.defaultMutationOptions(t),state:a});return this.add(n),n}add(e){this.#t.add(e);let t=Sl(e);if(typeof t=="string"){let a=this.#e.get(t);a?a.push(e):this.#e.set(t,[e])}this.notify({type:"added",mutation:e})}remove(e){if(this.#t.delete(e)){let t=Sl(e);if(typeof t=="string"){let a=this.#e.get(t);if(a)if(a.length>1){let n=a.indexOf(e);n!==-1&&a.splice(n,1)}else a[0]===e&&this.#e.delete(t)}}this.notify({type:"removed",mutation:e})}canRun(e){let t=Sl(e);if(typeof t=="string"){let n=this.#e.get(t)?.find(r=>r.state.status==="pending");return!n||n===e}else return!0}runNext(e){let t=Sl(e);return typeof t=="string"?this.#e.get(t)?.find(n=>n!==e&&n.state.isPaused)?.continue()??Promise.resolve():Promise.resolve()}clear(){ce.batch(()=>{this.#t.forEach(e=>{this.notify({type:"removed",mutation:e})}),this.#t.clear(),this.#e.clear()})}getAll(){return Array.from(this.#t)}find(e){let t={exact:!0,...e};return this.getAll().find(a=>yl(t,a))}findAll(e={}){return this.getAll().filter(t=>yl(e,t))}notify(e){ce.batch(()=>{this.listeners.forEach(t=>{t(e)})})}resumePausedMutations(){let e=this.getAll().filter(t=>t.state.isPaused);return ce.batch(()=>Promise.all(e.map(t=>t.continue().catch(De))))}};function Sl(e){return e.options.scope?.id}var _d=class extends Pt{#t;#e=void 0;#a;#n;constructor(e,t){super(),this.#t=e,this.setOptions(t),this.bindMethods(),this.#r()}bindMethods(){this.mutate=this.mutate.bind(this),this.reset=this.reset.bind(this)}setOptions(e){let t=this.options;this.options=this.#t.defaultMutationOptions(e),Tn(this.options,t)||this.#t.getMutationCache().notify({type:"observerOptionsUpdated",mutation:this.#a,observer:this}),t?.mutationKey&&this.options.mutationKey&&Oa(t.mutationKey)!==Oa(this.options.mutationKey)?this.reset():this.#a?.state.status==="pending"&&this.#a.setOptions(this.options)}onUnsubscribe(){this.hasListeners()||this.#a?.removeObserver(this)}onMutationUpdate(e){this.#r(),this.#s(e)}getCurrentResult(){return this.#e}reset(){this.#a?.removeObserver(this),this.#a=void 0,this.#r(),this.#s()}mutate(e,t){return this.#n=t,this.#a?.removeObserver(this),this.#a=this.#t.getMutationCache().build(this.#t,this.options),this.#a.addObserver(this),this.#a.execute(e)}#r(){let e=this.#a?.state??Nd();this.#e={...e,isPending:e.status==="pending",isSuccess:e.status==="success",isError:e.status==="error",isIdle:e.status==="idle",mutate:this.mutate,reset:this.reset}}#s(e){ce.batch(()=>{if(this.#n&&this.hasListeners()){let t=this.#e.variables,a=this.#e.context;e?.type==="success"?(this.#n.onSuccess?.(e.data,t,a),this.#n.onSettled?.(e.data,null,t,a)):e?.type==="error"&&(this.#n.onError?.(e.error,t,a),this.#n.onSettled?.(void 0,e.error,t,a))}this.listeners.forEach(t=>{t(this.#e)})})}};function uv(e,t){let a=new Set(t);return e.filter(n=>!a.has(n))}function bR(e,t,a){let n=e.slice(0);return n[t]=a,n}var kd=class extends Pt{#t;#e;#a;#n;#r;#s;#o;#i;#f=[];constructor(e,t,a){super(),this.#t=e,this.#n=a,this.#a=[],this.#r=[],this.#e=[],this.setQueries(t)}onSubscribe(){this.listeners.size===1&&this.#r.forEach(e=>{e.subscribe(t=>{this.#c(e,t)})})}onUnsubscribe(){this.listeners.size||this.destroy()}destroy(){this.listeners=new Set,this.#r.forEach(e=>{e.destroy()})}setQueries(e,t){this.#a=e,this.#n=t,ce.batch(()=>{let a=this.#r,n=this.#u(this.#a);this.#f=n,n.forEach(d=>d.observer.setOptions(d.defaultedQueryOptions));let r=n.map(d=>d.observer),s=r.map(d=>d.getCurrentResult()),i=a.length!==r.length,o=r.some((d,f)=>d!==a[f]),u=i||o,c=u?!0:s.some((d,f)=>{let m=this.#e[f];return!m||!Tn(d,m)});!u&&!c||(u&&(this.#r=r),this.#e=s,this.hasListeners()&&(u&&(uv(a,r).forEach(d=>{d.destroy()}),uv(r,a).forEach(d=>{d.subscribe(f=>{this.#c(d,f)})})),this.#l()))})}getCurrentResult(){return this.#e}getQueries(){return this.#r.map(e=>e.getCurrentQuery())}getObservers(){return this.#r}getOptimisticResult(e,t){let a=this.#u(e),n=a.map(r=>r.observer.getOptimisticResult(r.defaultedQueryOptions));return[n,r=>this.#m(r??n,t),()=>this.#d(n,a)]}#d(e,t){return t.map((a,n)=>{let r=e[n];return a.defaultedQueryOptions.notifyOnChangeProps?r:a.observer.trackResult(r,s=>{t.forEach(i=>{i.observer.trackProp(s)})})})}#m(e,t){return t?((!this.#s||this.#e!==this.#i||t!==this.#o)&&(this.#o=t,this.#i=this.#e,this.#s=Ri(this.#s,t(e))),this.#s):e}#u(e){let t=new Map(this.#r.map(n=>[n.options.queryHash,n])),a=[];return e.forEach(n=>{let r=this.#t.defaultQueryOptions(n),s=t.get(r.queryHash);s?a.push({defaultedQueryOptions:r,observer:s}):a.push({defaultedQueryOptions:r,observer:new hr(this.#t,r)})}),a}#c(e,t){let a=this.#r.indexOf(e);a!==-1&&(this.#e=bR(this.#e,a,t),this.#l())}#l(){if(this.hasListeners()){let e=this.#s,t=this.#d(this.#e,this.#f),a=this.#m(t,this.#n?.combine);e!==a&&ce.batch(()=>{this.listeners.forEach(n=>{n(this.#e)})})}}};var cv=class extends Pt{constructor(e={}){super(),this.config=e,this.#t=new Map}#t;build(e,t,a){let n=t.queryKey,r=t.queryHash??ki(n,t),s=this.get(r);return s||(s=new nv({client:e,queryKey:n,queryHash:r,options:e.defaultQueryOptions(t),state:a,defaultOptions:e.getQueryDefaults(n)}),this.add(s)),s}add(e){this.#t.has(e.queryHash)||(this.#t.set(e.queryHash,e),this.notify({type:"added",query:e}))}remove(e){let t=this.#t.get(e.queryHash);t&&(e.destroy(),t===e&&this.#t.delete(e.queryHash),this.notify({type:"removed",query:e}))}clear(){ce.batch(()=>{this.getAll().forEach(e=>{this.remove(e)})})}get(e){return this.#t.get(e)}getAll(){return[...this.#t.values()]}find(e){let t={exact:!0,...e};return this.getAll().find(a=>gl(t,a))}findAll(e={}){let t=this.getAll();return Object.keys(e).length>0?t.filter(a=>gl(e,a)):t}notify(e){ce.batch(()=>{this.listeners.forEach(t=>{t(e)})})}onFocus(){ce.batch(()=>{this.getAll().forEach(e=>{e.onFocus()})})}onOnline(){ce.batch(()=>{this.getAll().forEach(e=>{e.onOnline()})})}};var Rd=class{#t;#e;#a;#n;#r;#s;#o;#i;constructor(e={}){this.#t=e.queryCache||new cv,this.#e=e.mutationCache||new lv,this.#a=e.defaultOptions||{},this.#n=new Map,this.#r=new Map,this.#s=0}mount(){this.#s++,this.#s===1&&(this.#o=Jr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onFocus())}),this.#i=Xr.subscribe(async e=>{e&&(await this.resumePausedMutations(),this.#t.onOnline())}))}unmount(){this.#s--,this.#s===0&&(this.#o?.(),this.#o=void 0,this.#i?.(),this.#i=void 0)}isFetching(e){return this.#t.findAll({...e,fetchStatus:"fetching"}).length}isMutating(e){return this.#e.findAll({...e,status:"pending"}).length}getQueryData(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state.data}ensureQueryData(e){let t=this.defaultQueryOptions(e),a=this.#t.build(this,t),n=a.state.data;return n===void 0?this.fetchQuery(e):(e.revalidateIfStale&&a.isStaleByTime(Na(t.staleTime,a))&&this.prefetchQuery(t),Promise.resolve(n))}getQueriesData(e){return this.#t.findAll(e).map(({queryKey:t,state:a})=>{let n=a.data;return[t,n]})}setQueryData(e,t,a){let n=this.defaultQueryOptions({queryKey:e}),s=this.#t.get(n.queryHash)?.state.data,i=Xh(t,s);if(i!==void 0)return this.#t.build(this,n).setData(i,{...a,manual:!0})}setQueriesData(e,t,a){return ce.batch(()=>this.#t.findAll(e).map(({queryKey:n})=>[n,this.setQueryData(n,t,a)]))}getQueryState(e){let t=this.defaultQueryOptions({queryKey:e});return this.#t.get(t.queryHash)?.state}removeQueries(e){let t=this.#t;ce.batch(()=>{t.findAll(e).forEach(a=>{t.remove(a)})})}resetQueries(e,t){let a=this.#t;return ce.batch(()=>(a.findAll(e).forEach(n=>{n.reset()}),this.refetchQueries({type:"active",...e},t)))}cancelQueries(e,t={}){let a={revert:!0,...t},n=ce.batch(()=>this.#t.findAll(e).map(r=>r.cancel(a)));return Promise.all(n).then(De).catch(De)}invalidateQueries(e,t={}){return ce.batch(()=>(this.#t.findAll(e).forEach(a=>{a.invalidate()}),e?.refetchType==="none"?Promise.resolve():this.refetchQueries({...e,type:e?.refetchType??e?.type??"active"},t)))}refetchQueries(e,t={}){let a={...t,cancelRefetch:t.cancelRefetch??!0},n=ce.batch(()=>this.#t.findAll(e).filter(r=>!r.isDisabled()&&!r.isStatic()).map(r=>{let s=r.fetch(void 0,a);return a.throwOnError||(s=s.catch(De)),r.state.fetchStatus==="paused"?Promise.resolve():s}));return Promise.all(n).then(De)}fetchQuery(e){let t=this.defaultQueryOptions(e);t.retry===void 0&&(t.retry=!1);let a=this.#t.build(this,t);return a.isStaleByTime(Na(t.staleTime,a))?a.fetch(t):Promise.resolve(a.state.data)}prefetchQuery(e){return this.fetchQuery(e).then(De).catch(De)}fetchInfiniteQuery(e){return e.behavior=Sd(e.pages),this.fetchQuery(e)}prefetchInfiniteQuery(e){return this.fetchInfiniteQuery(e).then(De).catch(De)}ensureInfiniteQueryData(e){return e.behavior=Sd(e.pages),this.ensureQueryData(e)}resumePausedMutations(){return Xr.isOnline()?this.#e.resumePausedMutations():Promise.resolve()}getQueryCache(){return this.#t}getMutationCache(){return this.#e}getDefaultOptions(){return this.#a}setDefaultOptions(e){this.#a=e}setQueryDefaults(e,t){this.#n.set(Oa(e),{queryKey:e,defaultOptions:t})}getQueryDefaults(e){let t=[...this.#n.values()],a={};return t.forEach(n=>{pr(e,n.queryKey)&&Object.assign(a,n.defaultOptions)}),a}setMutationDefaults(e,t){this.#r.set(Oa(e),{mutationKey:e,defaultOptions:t})}getMutationDefaults(e){let t=[...this.#r.values()],a={};return t.forEach(n=>{pr(e,n.mutationKey)&&Object.assign(a,n.defaultOptions)}),a}defaultQueryOptions(e){if(e._defaulted)return e;let t={...this.#a.queries,...this.getQueryDefaults(e.queryKey),...e,_defaulted:!0};return t.queryHash||(t.queryHash=ki(t.queryKey,t)),t.refetchOnReconnect===void 0&&(t.refetchOnReconnect=t.networkMode!=="always"),t.throwOnError===void 0&&(t.throwOnError=!!t.suspense),!t.networkMode&&t.persister&&(t.networkMode="offlineFirst"),t.queryFn===Yr&&(t.enabled=!1),t}defaultMutationOptions(e){return e?._defaulted?e:{...this.#a.mutations,...e?.mutationKey&&this.getMutationDefaults(e.mutationKey),...e,_defaulted:!0}}clear(){this.#t.clear(),this.#e.clear()}};var La=ze(He(),1);var Zr=ze(He(),1),pv=ze(Cd(),1),Ed=Zr.createContext(void 0),J=e=>{let t=Zr.useContext(Ed);if(e)return e;if(!t)throw new Error("No QueryClient set, use QueryClientProvider to set one");return t},Td=({client:e,children:t})=>(Zr.useEffect(()=>(e.mount(),()=>{e.unmount()}),[e]),(0,pv.jsx)(Ed.Provider,{value:e,children:t}));var _l=ze(He(),1),hv=_l.createContext(!1),kl=()=>_l.useContext(hv),WL=hv.Provider;var Ai=ze(He(),1),wR=ze(Cd(),1);function SR(){let e=!1;return{clearReset:()=>{e=!1},reset:()=>{e=!0},isReset:()=>e}}var NR=Ai.createContext(SR()),Rl=()=>Ai.useContext(NR);var vv=ze(He(),1);var Cl=(e,t)=>{(e.suspense||e.throwOnError||e.experimental_prefetchInRender)&&(t.isReset()||(e.retryOnMount=!1))},El=e=>{vv.useEffect(()=>{e.clearReset()},[e])},Tl=({result:e,errorResetBoundary:t,throwOnError:a,query:n,suspense:r})=>e.isError&&!t.isReset()&&!e.isFetching&&n&&(r&&e.data===void 0||Ei(a,[e.error,n]));var Al=e=>{if(e.suspense){let a=r=>r==="static"?r:Math.max(r??1e3,1e3),n=e.staleTime;e.staleTime=typeof n=="function"?(...r)=>a(n(...r)):a(n),typeof e.gcTime=="number"&&(e.gcTime=Math.max(e.gcTime,1e3))}},Dl=(e,t)=>e.isLoading&&e.isFetching&&!t,Di=(e,t)=>e?.suspense&&t.isPending,Wr=(e,t,a)=>t.fetchOptimistic(e).catch(()=>{a.clearReset()});function Ad({queries:e,...t},a){let n=J(a),r=kl(),s=Rl(),i=La.useMemo(()=>e.map(y=>{let w=n.defaultQueryOptions(y);return w._optimisticResults=r?"isRestoring":"optimistic",w}),[e,n,r]);i.forEach(y=>{Al(y),Cl(y,s)}),El(s);let[o]=La.useState(()=>new kd(n,i,t)),[u,c,d]=o.getOptimisticResult(i,t.combine),f=!r&&t.subscribed!==!1;La.useSyncExternalStore(La.useCallback(y=>f?o.subscribe(ce.batchCalls(y)):De,[o,f]),()=>o.getCurrentResult(),()=>o.getCurrentResult()),La.useEffect(()=>{o.setQueries(i,t)},[i,t,o]);let h=u.some((y,w)=>Di(i[w],y))?u.flatMap((y,w)=>{let g=i[w];if(g){let v=new hr(n,g);if(Di(g,y))return Wr(g,v,s);Dl(y,r)&&Wr(g,v,s)}return[]}):[];if(h.length>0)throw Promise.all(h);let b=u.find((y,w)=>{let g=i[w];return g&&Tl({result:y,errorResetBoundary:s,throwOnError:g.throwOnError,query:n.getQueryCache().get(g.queryHash),suspense:g.suspense})});if(b?.error)throw b.error;return c(d())}var An=ze(He(),1);function gv(e,t,a){let n=kl(),r=Rl(),s=J(a),i=s.defaultQueryOptions(e);s.getDefaultOptions().queries?._experimental_beforeQuery?.(i),i._optimisticResults=n?"isRestoring":"optimistic",Al(i),Cl(i,r),El(r);let o=!s.getQueryCache().get(i.queryHash),[u]=An.useState(()=>new t(s,i)),c=u.getOptimisticResult(i),d=!n&&e.subscribed!==!1;if(An.useSyncExternalStore(An.useCallback(f=>{let m=d?u.subscribe(ce.batchCalls(f)):De;return u.updateResult(),m},[u,d]),()=>u.getCurrentResult(),()=>u.getCurrentResult()),An.useEffect(()=>{u.setOptions(i)},[i,u]),Di(i,c))throw Wr(i,u,r);if(Tl({result:c,errorResetBoundary:r,throwOnError:i.throwOnError,query:s.getQueryCache().get(i.queryHash),suspense:i.suspense}))throw c.error;return s.getDefaultOptions().queries?._experimental_afterQuery?.(i,c),i.experimental_prefetchInRender&&!Ut&&Dl(c,n)&&(o?Wr(i,u,r):s.getQueryCache().get(i.queryHash)?.promise)?.catch(De).finally(()=>{u.updateResult()}),i.notifyOnChangeProps?c:u.trackResult(c)}function K(e,t){return gv(e,hr,t)}var tn=ze(He(),1);function Q(e,t){let a=J(t),[n]=tn.useState(()=>new _d(a,e));tn.useEffect(()=>{n.setOptions(e)},[n,e]);let r=tn.useSyncExternalStore(tn.useCallback(i=>n.subscribe(ce.batchCalls(i)),[n]),()=>n.getCurrentResult(),()=>n.getCurrentResult()),s=tn.useCallback((i,o)=>{n.mutate(i,o).catch(De)},[n]);if(r.error&&Ei(n.options.throwOnError,[r.error]))throw r.error;return{...r,mutate:s,mutateAsync:r.mutate}}var lR=ze(F0());var ra=ze(He(),1),X=ze(He(),1),Ce=ze(He(),1),_p=ze(He(),1),lx=ze(He(),1),ye=ze(He(),1),eT=ze(He(),1),tT=ze(He(),1),aT=ze(He(),1),W=ze(He(),1),Sx=ze(He(),1);var B0="popstate";function H0(e={}){function t(n,r){let{pathname:s,search:i,hash:o}=n.location;return up("",{pathname:s,search:i,hash:o},r.state&&r.state.usr||null,r.state&&r.state.key||"default")}function a(n,r){return typeof r=="string"?r:Hs(r)}return t3(t,a,null,e)}function Re(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}function na(e,t){if(!e){typeof console<"u"&&console.warn(t);try{throw new Error(t)}catch{}}}function e3(){return Math.random().toString(36).substring(2,10)}function z0(e,t){return{usr:e.state,key:e.key,idx:t}}function up(e,t,a=null,n){return{pathname:typeof e=="string"?e:e.pathname,search:"",hash:"",...typeof t=="string"?Lr(t):t,state:a,key:t&&t.key||n||e3()}}function Hs({pathname:e="/",search:t="",hash:a=""}){return t&&t!=="?"&&(e+=t.charAt(0)==="?"?t:"?"+t),a&&a!=="#"&&(e+=a.charAt(0)==="#"?a:"#"+a),e}function Lr(e){let t={};if(e){let a=e.indexOf("#");a>=0&&(t.hash=e.substring(a),e=e.substring(0,a));let n=e.indexOf("?");n>=0&&(t.search=e.substring(n),e=e.substring(0,n)),e&&(t.pathname=e)}return t}function t3(e,t,a,n={}){let{window:r=document.defaultView,v5Compat:s=!1}=n,i=r.history,o="POP",u=null,c=d();c==null&&(c=0,i.replaceState({...i.state,idx:c},""));function d(){return(i.state||{idx:null}).idx}function f(){o="POP";let w=d(),g=w==null?null:w-c;c=w,u&&u({action:o,location:y.location,delta:g})}function m(w,g){o="PUSH";let v=up(y.location,w,g);a&&a(v,w),c=d()+1;let x=z0(v,c),$=y.createHref(v);try{i.pushState(x,"",$)}catch(S){if(S instanceof DOMException&&S.name==="DataCloneError")throw S;r.location.assign($)}s&&u&&u({action:o,location:y.location,delta:1})}function h(w,g){o="REPLACE";let v=up(y.location,w,g);a&&a(v,w),c=d();let x=z0(v,c),$=y.createHref(v);i.replaceState(x,"",$),s&&u&&u({action:o,location:y.location,delta:0})}function b(w){return a3(w)}let y={get action(){return o},get location(){return e(r,i)},listen(w){if(u)throw new Error("A history only accepts one active listener");return r.addEventListener(B0,f),u=w,()=>{r.removeEventListener(B0,f),u=null}},createHref(w){return t(r,w)},createURL:b,encodeLocation(w){let g=b(w);return{pathname:g.pathname,search:g.search,hash:g.hash}},push:m,replace:h,go(w){return i.go(w)}};return y}function a3(e,t=!1){let a="http://localhost";typeof window<"u"&&(a=window.location.origin!=="null"?window.location.origin:window.location.href),Re(a,"No window.location.(origin|href) available to create URL");let n=typeof e=="string"?e:Hs(e);return n=n.replace(/ $/,"%20"),!t&&n.startsWith("//")&&(n=a+n),new URL(n,a)}var n3;n3=new WeakMap;function fp(e,t,a="/"){return r3(e,t,a,!1)}function r3(e,t,a,n){let r=typeof t=="string"?Lr(t):t,s=Ha(r.pathname||"/",a);if(s==null)return null;let i=Q0(e);i3(i);let o=null;for(let u=0;o==null&&u<i.length;++u){let c=g3(s);o=h3(i[u],c,n)}return o}function s3(e,t){let{route:a,pathname:n,params:r}=e;return{id:a.id,pathname:n,params:r,data:t[a.id],loaderData:t[a.id],handle:a.handle}}function Q0(e,t=[],a=[],n="",r=!1){let s=(i,o,u=r,c)=>{let d={relativePath:c===void 0?i.path||"":c,caseSensitive:i.caseSensitive===!0,childrenIndex:o,route:i};if(d.relativePath.startsWith("/")){if(!d.relativePath.startsWith(n)&&u)return;Re(d.relativePath.startsWith(n),`Absolute route path "${d.relativePath}" nested under path "${n}" is not valid. An absolute child route path must start with the combined path of all its parent routes.`),d.relativePath=d.relativePath.slice(n.length)}let f=xn([n,d.relativePath]),m=a.concat(d);i.children&&i.children.length>0&&(Re(i.index!==!0,`Index routes must not have child routes. Please remove all child routes from route path "${f}".`),Q0(i.children,t,m,f,u)),!(i.path==null&&!i.index)&&t.push({path:f,score:f3(f,i.index),routesMeta:m})};return e.forEach((i,o)=>{if(i.path===""||!i.path?.includes("?"))s(i,o);else for(let u of V0(i.path))s(i,o,!0,u)}),t}function V0(e){let t=e.split("/");if(t.length===0)return[];let[a,...n]=t,r=a.endsWith("?"),s=a.replace(/\?$/,"");if(n.length===0)return r?[s,""]:[s];let i=V0(n.join("/")),o=[];return o.push(...i.map(u=>u===""?s:[s,u].join("/"))),r&&o.push(...i),o.map(u=>e.startsWith("/")&&u===""?"/":u)}function i3(e){e.sort((t,a)=>t.score!==a.score?a.score-t.score:p3(t.routesMeta.map(n=>n.childrenIndex),a.routesMeta.map(n=>n.childrenIndex)))}var o3=/^:[\w-]+$/,l3=3,u3=2,c3=1,d3=10,m3=-2,q0=e=>e==="*";function f3(e,t){let a=e.split("/"),n=a.length;return a.some(q0)&&(n+=m3),t&&(n+=u3),a.filter(r=>!q0(r)).reduce((r,s)=>r+(o3.test(s)?l3:s===""?c3:d3),n)}function p3(e,t){return e.length===t.length&&e.slice(0,-1).every((n,r)=>n===t[r])?e[e.length-1]-t[t.length-1]:0}function h3(e,t,a=!1){let{routesMeta:n}=e,r={},s="/",i=[];for(let o=0;o<n.length;++o){let u=n[o],c=o===n.length-1,d=s==="/"?t:t.slice(s.length)||"/",f=zo({path:u.relativePath,caseSensitive:u.caseSensitive,end:c},d),m=u.route;if(!f&&c&&a&&!n[n.length-1].route.index&&(f=zo({path:u.relativePath,caseSensitive:u.caseSensitive,end:!1},d)),!f)return null;Object.assign(r,f.params),i.push({params:r,pathname:xn([s,f.pathname]),pathnameBase:x3(xn([s,f.pathnameBase])),route:m}),f.pathnameBase!=="/"&&(s=xn([s,f.pathnameBase]))}return i}function zo(e,t){typeof e=="string"&&(e={path:e,caseSensitive:!1,end:!0});let[a,n]=v3(e.path,e.caseSensitive,e.end),r=t.match(a);if(!r)return null;let s=r[0],i=s.replace(/(.)\/+$/,"$1"),o=r.slice(1);return{params:n.reduce((c,{paramName:d,isOptional:f},m)=>{if(d==="*"){let b=o[m]||"";i=s.slice(0,s.length-b.length).replace(/(.)\/+$/,"$1")}let h=o[m];return f&&!h?c[d]=void 0:c[d]=(h||"").replace(/%2F/g,"/"),c},{}),pathname:s,pathnameBase:i,pattern:e}}function v3(e,t=!1,a=!0){na(e==="*"||!e.endsWith("*")||e.endsWith("/*"),`Route path "${e}" will be treated as if it were "${e.replace(/\*$/,"/*")}" because the \`*\` character must always follow a \`/\` in the pattern. To get rid of this warning, please change the route path to "${e.replace(/\*$/,"/*")}".`);let n=[],r="^"+e.replace(/\/*\*?$/,"").replace(/^\/*/,"/").replace(/[\\.*+^${}|()[\]]/g,"\\$&").replace(/\/:([\w-]+)(\?)?/g,(i,o,u)=>(n.push({paramName:o,isOptional:u!=null}),u?"/?([^\\/]+)?":"/([^\\/]+)")).replace(/\/([\w-]+)\?(\/|$)/g,"(/$1)?$2");return e.endsWith("*")?(n.push({paramName:"*"}),r+=e==="*"||e==="/*"?"(.*)$":"(?:\\/(.+)|\\/*)$"):a?r+="\\/*$":e!==""&&e!=="/"&&(r+="(?:(?=\\/|$))"),[new RegExp(r,t?void 0:"i"),n]}function g3(e){try{return e.split("/").map(t=>decodeURIComponent(t).replace(/\//g,"%2F")).join("/")}catch(t){return na(!1,`The URL path "${e}" could not be decoded because it is a malformed URL segment. This is probably due to a bad percent encoding (${t}).`),e}}function Ha(e,t){if(t==="/")return e;if(!e.toLowerCase().startsWith(t.toLowerCase()))return null;let a=t.endsWith("/")?t.length-1:t.length,n=e.charAt(a);return n&&n!=="/"?null:e.slice(a)||"/"}function G0(e,t="/"){let{pathname:a,search:n="",hash:r=""}=typeof e=="string"?Lr(e):e;return{pathname:a?a.startsWith("/")?a:y3(a,t):t,search:$3(n),hash:w3(r)}}function y3(e,t){let a=t.replace(/\/+$/,"").split("/");return e.split("/").forEach(r=>{r===".."?a.length>1&&a.pop():r!=="."&&a.push(r)}),a.length>1?a.join("/"):"/"}function op(e,t,a,n){return`Cannot include a '${e}' character in a manually specified \`to.${t}\` field [${JSON.stringify(n)}].  Please separate it out to the \`to.${a}\` field. Alternatively you may provide the full path as a string in <Link to="..."> and the router will parse it for you.`}function b3(e){return e.filter((t,a)=>a===0||t.route.path&&t.route.path.length>0)}function pp(e){let t=b3(e);return t.map((a,n)=>n===t.length-1?a.pathname:a.pathnameBase)}function hp(e,t,a,n=!1){let r;typeof e=="string"?r=Lr(e):(r={...e},Re(!r.pathname||!r.pathname.includes("?"),op("?","pathname","search",r)),Re(!r.pathname||!r.pathname.includes("#"),op("#","pathname","hash",r)),Re(!r.search||!r.search.includes("#"),op("#","search","hash",r)));let s=e===""||r.pathname==="",i=s?"/":r.pathname,o;if(i==null)o=a;else{let f=t.length-1;if(!n&&i.startsWith("..")){let m=i.split("/");for(;m[0]==="..";)m.shift(),f-=1;r.pathname=m.join("/")}o=f>=0?t[f]:"/"}let u=G0(r,o),c=i&&i!=="/"&&i.endsWith("/"),d=(s||i===".")&&a.endsWith("/");return!u.pathname.endsWith("/")&&(c||d)&&(u.pathname+="/"),u}var xn=e=>e.join("/").replace(/\/\/+/g,"/"),x3=e=>e.replace(/\/+$/,"").replace(/^\/*/,"/"),$3=e=>!e||e==="?"?"":e.startsWith("?")?e:"?"+e,w3=e=>!e||e==="#"?"":e.startsWith("#")?e:"#"+e;function Y0(e){return e!=null&&typeof e.status=="number"&&typeof e.statusText=="string"&&typeof e.internal=="boolean"&&"data"in e}var J0=["POST","PUT","PATCH","DELETE"],j6=new Set(J0),S3=["GET",...J0],F6=new Set(S3);var B6=Symbol("ResetLoaderData");var Pr=ra.createContext(null);Pr.displayName="DataRouter";var Qs=ra.createContext(null);Qs.displayName="DataRouterState";var z6=ra.createContext(!1);var vp=ra.createContext({isTransitioning:!1});vp.displayName="ViewTransition";var X0=ra.createContext(new Map);X0.displayName="Fetchers";var N3=ra.createContext(null);N3.displayName="Await";var Kt=ra.createContext(null);Kt.displayName="Navigation";var Vs=ra.createContext(null);Vs.displayName="Location";var sa=ra.createContext({outlet:null,matches:[],isDataRoute:!1});sa.displayName="Route";var gp=ra.createContext(null);gp.displayName="RouteError";var cp=!0;function Z0(e,{relative:t}={}){Re(Ur(),"useHref() may be used only in the context of a <Router> component.");let{basename:a,navigator:n}=X.useContext(Kt),{hash:r,pathname:s,search:i}=Gs(e,{relative:t}),o=s;return a!=="/"&&(o=s==="/"?a:xn([a,s])),n.createHref({pathname:o,search:i,hash:r})}function Ur(){return X.useContext(Vs)!=null}function Pe(){return Re(Ur(),"useLocation() may be used only in the context of a <Router> component."),X.useContext(Vs).location}var W0="You should call navigate() in a React.useEffect(), not when your component is first rendered.";function ex(e){X.useContext(Kt).static||X.useLayoutEffect(e)}function fe(){let{isDataRoute:e}=X.useContext(sa);return e?O3():_3()}function _3(){Re(Ur(),"useNavigate() may be used only in the context of a <Router> component.");let e=X.useContext(Pr),{basename:t,navigator:a}=X.useContext(Kt),{matches:n}=X.useContext(sa),{pathname:r}=Pe(),s=JSON.stringify(pp(n)),i=X.useRef(!1);return ex(()=>{i.current=!0}),X.useCallback((u,c={})=>{if(na(i.current,W0),!i.current)return;if(typeof u=="number"){a.go(u);return}let d=hp(u,JSON.parse(s),r,c.relative==="path");e==null&&t!=="/"&&(d.pathname=d.pathname==="/"?t:xn([t,d.pathname])),(c.replace?a.replace:a.push)(d,c.state,c)},[t,a,s,r,e])}var tx=X.createContext(null);function wa(){return X.useContext(tx)}function ax(e){let t=X.useContext(sa).outlet;return t&&X.createElement(tx.Provider,{value:e},t)}function it(){let{matches:e}=X.useContext(sa),t=e[e.length-1];return t?t.params:{}}function Gs(e,{relative:t}={}){let{matches:a}=X.useContext(sa),{pathname:n}=Pe(),r=JSON.stringify(pp(a));return X.useMemo(()=>hp(e,JSON.parse(r),n,t==="path"),[e,r,n,t])}function nx(e,t){return rx(e,t)}function rx(e,t,a,n,r){Re(Ur(),"useRoutes() may be used only in the context of a <Router> component.");let{navigator:s}=X.useContext(Kt),{matches:i}=X.useContext(sa),o=i[i.length-1],u=o?o.params:{},c=o?o.pathname:"/",d=o?o.pathnameBase:"/",f=o&&o.route;if(cp){let v=f&&f.path||"";ox(c,!f||v.endsWith("*")||v.endsWith("*?"),`You rendered descendant <Routes> (or called \`useRoutes()\`) at "${c}" (under <Route path="${v}">) but the parent route path has no trailing "*". This means if you navigate deeper, the parent won't match anymore and therefore the child routes will never render.

Please change the parent <Route path="${v}"> to <Route path="${v==="/"?"*":`${v}/*`}">.`)}let m=Pe(),h;if(t){let v=typeof t=="string"?Lr(t):t;Re(d==="/"||v.pathname?.startsWith(d),`When overriding the location using \`<Routes location>\` or \`useRoutes(routes, location)\`, the location pathname must begin with the portion of the URL pathname that was matched by all parent routes. The current pathname base is "${d}" but pathname "${v.pathname}" was given in the \`location\` prop.`),h=v}else h=m;let b=h.pathname||"/",y=b;if(d!=="/"){let v=d.replace(/^\//,"").split("/");y="/"+b.replace(/^\//,"").split("/").slice(v.length).join("/")}let w=fp(e,{pathname:y});cp&&(na(f||w!=null,`No routes matched location "${h.pathname}${h.search}${h.hash}" `),na(w==null||w[w.length-1].route.element!==void 0||w[w.length-1].route.Component!==void 0||w[w.length-1].route.lazy!==void 0,`Matched leaf route at location "${h.pathname}${h.search}${h.hash}" does not have an element or Component. This means it will render an <Outlet /> with a null value by default resulting in an "empty" page.`));let g=T3(w&&w.map(v=>Object.assign({},v,{params:Object.assign({},u,v.params),pathname:xn([d,s.encodeLocation?s.encodeLocation(v.pathname).pathname:v.pathname]),pathnameBase:v.pathnameBase==="/"?d:xn([d,s.encodeLocation?s.encodeLocation(v.pathnameBase).pathname:v.pathnameBase])})),i,a,n,r);return t&&g?X.createElement(Vs.Provider,{value:{location:{pathname:"/",search:"",hash:"",state:null,key:"default",...h},navigationType:"POP"}},g):g}function k3(){let e=ix(),t=Y0(e)?`${e.status} ${e.statusText}`:e instanceof Error?e.message:JSON.stringify(e),a=e instanceof Error?e.stack:null,n="rgba(200,200,200, 0.5)",r={padding:"0.5rem",backgroundColor:n},s={padding:"2px 4px",backgroundColor:n},i=null;return cp&&(console.error("Error handled by React Router default ErrorBoundary:",e),i=X.createElement(X.Fragment,null,X.createElement("p",null,"\u{1F4BF} Hey developer \u{1F44B}"),X.createElement("p",null,"You can provide a way better UX than this when your app throws errors by providing your own ",X.createElement("code",{style:s},"ErrorBoundary")," or"," ",X.createElement("code",{style:s},"errorElement")," prop on your route."))),X.createElement(X.Fragment,null,X.createElement("h2",null,"Unexpected Application Error!"),X.createElement("h3",{style:{fontStyle:"italic"}},t),a?X.createElement("pre",{style:r},a):null,i)}var R3=X.createElement(k3,null),C3=class extends X.Component{constructor(e){super(e),this.state={location:e.location,revalidation:e.revalidation,error:e.error}}static getDerivedStateFromError(e){return{error:e}}static getDerivedStateFromProps(e,t){return t.location!==e.location||t.revalidation!=="idle"&&e.revalidation==="idle"?{error:e.error,location:e.location,revalidation:e.revalidation}:{error:e.error!==void 0?e.error:t.error,location:t.location,revalidation:e.revalidation||t.revalidation}}componentDidCatch(e,t){this.props.unstable_onError?this.props.unstable_onError(e,t):console.error("React Router caught the following error during render",e)}render(){return this.state.error!==void 0?X.createElement(sa.Provider,{value:this.props.routeContext},X.createElement(gp.Provider,{value:this.state.error,children:this.props.component})):this.props.children}};function E3({routeContext:e,match:t,children:a}){let n=X.useContext(Pr);return n&&n.static&&n.staticContext&&(t.route.errorElement||t.route.ErrorBoundary)&&(n.staticContext._deepestRenderedBoundaryId=t.route.id),X.createElement(sa.Provider,{value:e},a)}function T3(e,t=[],a=null,n=null,r=null){if(e==null){if(!a)return null;if(a.errors)e=a.matches;else if(t.length===0&&!a.initialized&&a.matches.length>0)e=a.matches;else return null}let s=e,i=a?.errors;if(i!=null){let c=s.findIndex(d=>d.route.id&&i?.[d.route.id]!==void 0);Re(c>=0,`Could not find a matching route for errors on route IDs: ${Object.keys(i).join(",")}`),s=s.slice(0,Math.min(s.length,c+1))}let o=!1,u=-1;if(a)for(let c=0;c<s.length;c++){let d=s[c];if((d.route.HydrateFallback||d.route.hydrateFallbackElement)&&(u=c),d.route.id){let{loaderData:f,errors:m}=a,h=d.route.loader&&!f.hasOwnProperty(d.route.id)&&(!m||m[d.route.id]===void 0);if(d.route.lazy||h){o=!0,u>=0?s=s.slice(0,u+1):s=[s[0]];break}}}return s.reduceRight((c,d,f)=>{let m,h=!1,b=null,y=null;a&&(m=i&&d.route.id?i[d.route.id]:void 0,b=d.route.errorElement||R3,o&&(u<0&&f===0?(ox("route-fallback",!1,"No `HydrateFallback` element provided to render during initial hydration"),h=!0,y=null):u===f&&(h=!0,y=d.route.hydrateFallbackElement||null)));let w=t.concat(s.slice(0,f+1)),g=()=>{let v;return m?v=b:h?v=y:d.route.Component?v=X.createElement(d.route.Component,null):d.route.element?v=d.route.element:v=c,X.createElement(E3,{match:d,routeContext:{outlet:c,matches:w,isDataRoute:a!=null},children:v})};return a&&(d.route.ErrorBoundary||d.route.errorElement||f===0)?X.createElement(C3,{location:a.location,revalidation:a.revalidation,component:b,error:m,children:g(),routeContext:{outlet:null,matches:w,isDataRoute:!0},unstable_onError:n}):g()},null)}function yp(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function A3(e){let t=X.useContext(Pr);return Re(t,yp(e)),t}function bp(e){let t=X.useContext(Qs);return Re(t,yp(e)),t}function D3(e){let t=X.useContext(sa);return Re(t,yp(e)),t}function xp(e){let t=D3(e),a=t.matches[t.matches.length-1];return Re(a.route.id,`${e} can only be used on routes that contain a unique "id"`),a.route.id}function M3(){return xp("useRouteId")}function sx(){return bp("useNavigation").navigation}function $p(){let{matches:e,loaderData:t}=bp("useMatches");return X.useMemo(()=>e.map(a=>s3(a,t)),[e,t])}function ix(){let e=X.useContext(gp),t=bp("useRouteError"),a=xp("useRouteError");return e!==void 0?e:t.errors?.[a]}function O3(){let{router:e}=A3("useNavigate"),t=xp("useNavigate"),a=X.useRef(!1);return ex(()=>{a.current=!0}),X.useCallback(async(r,s={})=>{na(a.current,W0),a.current&&(typeof r=="number"?e.navigate(r):await e.navigate(r,{fromRouteId:t,...s}))},[e,t])}var I0={};function ox(e,t,a){!t&&!I0[e]&&(I0[e]=!0,na(!1,a))}var q6=Ce.memo(L3);function L3({routes:e,future:t,state:a,unstable_onError:n}){return rx(e,void 0,a,n,t)}function ot({to:e,replace:t,state:a,relative:n}){Re(Ur(),"<Navigate> may be used only in the context of a <Router> component.");let{static:r}=Ce.useContext(Kt);na(!r,"<Navigate> must not be used on the initial render in a <StaticRouter>. This is a no-op, but you should modify your code so the <Navigate> is only ever rendered in response to some user interaction or state change.");let{matches:s}=Ce.useContext(sa),{pathname:i}=Pe(),o=fe(),u=hp(e,pp(s),i,n==="path"),c=JSON.stringify(u);return Ce.useEffect(()=>{o(JSON.parse(c),{replace:t,state:a,relative:n})},[o,c,n,t,a]),null}function wp(e){return ax(e.context)}function be(e){Re(!1,"A <Route> is only ever to be used as the child of <Routes> element, never rendered directly. Please wrap your <Route> in a <Routes>.")}function Sp({basename:e="/",children:t=null,location:a,navigationType:n="POP",navigator:r,static:s=!1}){Re(!Ur(),"You cannot render a <Router> inside another <Router>. You should never have more than one in your app.");let i=e.replace(/^\/*/,"/"),o=Ce.useMemo(()=>({basename:i,navigator:r,static:s,future:{}}),[i,r,s]);typeof a=="string"&&(a=Lr(a));let{pathname:u="/",search:c="",hash:d="",state:f=null,key:m="default"}=a,h=Ce.useMemo(()=>{let b=Ha(u,i);return b==null?null:{location:{pathname:b,search:c,hash:d,state:f,key:m},navigationType:n}},[i,u,c,d,f,m,n]);return na(h!=null,`<Router basename="${i}"> is not able to match the URL "${u}${c}${d}" because it does not start with the basename, so the <Router> won't render anything.`),h==null?null:Ce.createElement(Kt.Provider,{value:o},Ce.createElement(Vs.Provider,{children:t,value:h}))}function Np({children:e,location:t}){return nx(uc(e),t)}function uc(e,t=[]){let a=[];return Ce.Children.forEach(e,(n,r)=>{if(!Ce.isValidElement(n))return;let s=[...t,r];if(n.type===Ce.Fragment){a.push.apply(a,uc(n.props.children,s));return}Re(n.type===be,`[${typeof n.type=="string"?n.type:n.type.name}] is not a <Route> component. All component children of <Routes> must be a <Route> or <React.Fragment>`),Re(!n.props.index||!n.props.children,"An index route cannot have child routes.");let i={id:n.props.id||s.join("-"),caseSensitive:n.props.caseSensitive,element:n.props.element,Component:n.props.Component,index:n.props.index,path:n.props.path,loader:n.props.loader,action:n.props.action,hydrateFallbackElement:n.props.hydrateFallbackElement,HydrateFallback:n.props.HydrateFallback,errorElement:n.props.errorElement,ErrorBoundary:n.props.ErrorBoundary,hasErrorBoundary:n.props.hasErrorBoundary===!0||n.props.ErrorBoundary!=null||n.props.errorElement!=null,shouldRevalidate:n.props.shouldRevalidate,handle:n.props.handle,lazy:n.props.lazy};n.props.children&&(i.children=uc(n.props.children,s)),a.push(i)}),a}var oc="get",lc="application/x-www-form-urlencoded";function cc(e){return e!=null&&typeof e.tagName=="string"}function P3(e){return cc(e)&&e.tagName.toLowerCase()==="button"}function U3(e){return cc(e)&&e.tagName.toLowerCase()==="form"}function j3(e){return cc(e)&&e.tagName.toLowerCase()==="input"}function F3(e){return!!(e.metaKey||e.altKey||e.ctrlKey||e.shiftKey)}function B3(e,t){return e.button===0&&(!t||t==="_self")&&!F3(e)}var sc=null;function z3(){if(sc===null)try{new FormData(document.createElement("form"),0),sc=!1}catch{sc=!0}return sc}var q3=new Set(["application/x-www-form-urlencoded","multipart/form-data","text/plain"]);function lp(e){return e!=null&&!q3.has(e)?(na(!1,`"${e}" is not a valid \`encType\` for \`<Form>\`/\`<fetcher.Form>\` and will default to "${lc}"`),null):e}function I3(e,t){let a,n,r,s,i;if(U3(e)){let o=e.getAttribute("action");n=o?Ha(o,t):null,a=e.getAttribute("method")||oc,r=lp(e.getAttribute("enctype"))||lc,s=new FormData(e)}else if(P3(e)||j3(e)&&(e.type==="submit"||e.type==="image")){let o=e.form;if(o==null)throw new Error('Cannot submit a <button> or <input type="submit"> without a <form>');let u=e.getAttribute("formaction")||o.getAttribute("action");if(n=u?Ha(u,t):null,a=e.getAttribute("formmethod")||o.getAttribute("method")||oc,r=lp(e.getAttribute("formenctype"))||lp(o.getAttribute("enctype"))||lc,s=new FormData(o,e),!z3()){let{name:c,type:d,value:f}=e;if(d==="image"){let m=c?`${c}.`:"";s.append(`${m}x`,"0"),s.append(`${m}y`,"0")}else c&&s.append(c,f)}}else{if(cc(e))throw new Error('Cannot submit element that is not <form>, <button>, or <input type="submit|image">');a=oc,n=null,r=lc,i=e}return s&&r==="text/plain"&&(i=s,s=void 0),{action:n,method:a.toLowerCase(),encType:r,formData:s,body:i}}var I6=Object.getOwnPropertyNames(Object.prototype).sort().join("\0");function kp(e,t){if(e===!1||e===null||typeof e>"u")throw new Error(t)}var K3=Symbol("SingleFetchRedirect");function H3(e,t,a){let n=typeof e=="string"?new URL(e,typeof window>"u"?"server://singlefetch/":window.location.origin):e;return n.pathname==="/"?n.pathname=`_root.${a}`:t&&Ha(n.pathname,t)==="/"?n.pathname=`${t.replace(/\/$/,"")}/_root.${a}`:n.pathname=`${n.pathname.replace(/\/$/,"")}.${a}`,n}async function Q3(e,t){if(e.id in t)return t[e.id];try{let a=await import(e.module);return t[e.id]=a,a}catch(a){if(console.error(`Error loading route module \`${e.module}\`, reloading page...`),console.error(a),window.__reactRouterContext&&window.__reactRouterContext.isSpaMode&&import.meta.hot)throw a;return window.location.reload(),new Promise(()=>{})}}function V3(e){return e!=null&&typeof e.page=="string"}function G3(e){return e==null?!1:e.href==null?e.rel==="preload"&&typeof e.imageSrcSet=="string"&&typeof e.imageSizes=="string":typeof e.rel=="string"&&typeof e.href=="string"}async function Y3(e,t,a){let n=await Promise.all(e.map(async r=>{let s=t.routes[r.route.id];if(s){let i=await Q3(s,a);return i.links?i.links():[]}return[]}));return W3(n.flat(1).filter(G3).filter(r=>r.rel==="stylesheet"||r.rel==="preload").map(r=>r.rel==="stylesheet"?{...r,rel:"prefetch",as:"style"}:{...r,rel:"prefetch"}))}function K0(e,t,a,n,r,s){let i=(u,c)=>a[c]?u.route.id!==a[c].route.id:!0,o=(u,c)=>a[c].pathname!==u.pathname||a[c].route.path?.endsWith("*")&&a[c].params["*"]!==u.params["*"];return s==="assets"?t.filter((u,c)=>i(u,c)||o(u,c)):s==="data"?t.filter((u,c)=>{let d=n.routes[u.route.id];if(!d||!d.hasLoader)return!1;if(i(u,c)||o(u,c))return!0;if(u.route.shouldRevalidate){let f=u.route.shouldRevalidate({currentUrl:new URL(r.pathname+r.search+r.hash,window.origin),currentParams:a[0]?.params||{},nextUrl:new URL(e,window.origin),nextParams:u.params,defaultShouldRevalidate:!0});if(typeof f=="boolean")return f}return!0}):[]}function J3(e,t,{includeHydrateFallback:a}={}){return X3(e.map(n=>{let r=t.routes[n.route.id];if(!r)return[];let s=[r.module];return r.clientActionModule&&(s=s.concat(r.clientActionModule)),r.clientLoaderModule&&(s=s.concat(r.clientLoaderModule)),a&&r.hydrateFallbackModule&&(s=s.concat(r.hydrateFallbackModule)),r.imports&&(s=s.concat(r.imports)),s}).flat(1))}function X3(e){return[...new Set(e)]}function Z3(e){let t={},a=Object.keys(e).sort();for(let n of a)t[n]=e[n];return t}function W3(e,t){let a=new Set,n=new Set(t);return e.reduce((r,s)=>{if(t&&!V3(s)&&s.as==="script"&&s.href&&n.has(s.href))return r;let o=JSON.stringify(Z3(s));return a.has(o)||(a.add(o),r.push({key:o,link:s})),r},[])}function ux(){let e=ye.useContext(Pr);return kp(e,"You must render this element inside a <DataRouterContext.Provider> element"),e}function nT(){let e=ye.useContext(Qs);return kp(e,"You must render this element inside a <DataRouterStateContext.Provider> element"),e}var qo=ye.createContext(void 0);qo.displayName="FrameworkContext";function cx(){let e=ye.useContext(qo);return kp(e,"You must render this element inside a <HydratedRouter> element"),e}function rT(e,t){let a=ye.useContext(qo),[n,r]=ye.useState(!1),[s,i]=ye.useState(!1),{onFocus:o,onBlur:u,onMouseEnter:c,onMouseLeave:d,onTouchStart:f}=t,m=ye.useRef(null);ye.useEffect(()=>{if(e==="render"&&i(!0),e==="viewport"){let y=g=>{g.forEach(v=>{i(v.isIntersecting)})},w=new IntersectionObserver(y,{threshold:.5});return m.current&&w.observe(m.current),()=>{w.disconnect()}}},[e]),ye.useEffect(()=>{if(n){let y=setTimeout(()=>{i(!0)},100);return()=>{clearTimeout(y)}}},[n]);let h=()=>{r(!0)},b=()=>{r(!1),i(!1)};return a?e!=="intent"?[s,m,{}]:[s,m,{onFocus:Bo(o,h),onBlur:Bo(u,b),onMouseEnter:Bo(c,h),onMouseLeave:Bo(d,b),onTouchStart:Bo(f,h)}]:[!1,m,{}]}function Bo(e,t){return a=>{e&&e(a),a.defaultPrevented||t(a)}}function dx({page:e,...t}){let{router:a}=ux(),n=ye.useMemo(()=>fp(a.routes,e,a.basename),[a.routes,e,a.basename]);return n?ye.createElement(iT,{page:e,matches:n,...t}):null}function sT(e){let{manifest:t,routeModules:a}=cx(),[n,r]=ye.useState([]);return ye.useEffect(()=>{let s=!1;return Y3(e,t,a).then(i=>{s||r(i)}),()=>{s=!0}},[e,t,a]),n}function iT({page:e,matches:t,...a}){let n=Pe(),{manifest:r,routeModules:s}=cx(),{basename:i}=ux(),{loaderData:o,matches:u}=nT(),c=ye.useMemo(()=>K0(e,t,u,r,n,"data"),[e,t,u,r,n]),d=ye.useMemo(()=>K0(e,t,u,r,n,"assets"),[e,t,u,r,n]),f=ye.useMemo(()=>{if(e===n.pathname+n.search+n.hash)return[];let b=new Set,y=!1;if(t.forEach(g=>{let v=r.routes[g.route.id];!v||!v.hasLoader||(!c.some(x=>x.route.id===g.route.id)&&g.route.id in o&&s[g.route.id]?.shouldRevalidate||v.hasClientLoader?y=!0:b.add(g.route.id))}),b.size===0)return[];let w=H3(e,i,"data");return y&&b.size>0&&w.searchParams.set("_routes",t.filter(g=>b.has(g.route.id)).map(g=>g.route.id).join(",")),[w.pathname+w.search]},[i,o,n,r,c,t,e,s]),m=ye.useMemo(()=>J3(d,r),[d,r]),h=sT(d);return ye.createElement(ye.Fragment,null,f.map(b=>ye.createElement("link",{key:b,rel:"prefetch",as:"fetch",href:b,...a})),m.map(b=>ye.createElement("link",{key:b,rel:"modulepreload",href:b,...a})),h.map(({key:b,link:y})=>ye.createElement("link",{key:b,nonce:a.nonce,...y})))}function oT(...e){return t=>{e.forEach(a=>{typeof a=="function"?a(t):a!=null&&(a.current=t)})}}var mx=typeof window<"u"&&typeof window.document<"u"&&typeof window.document.createElement<"u";try{mx&&(window.__reactRouterVersion="7.9.1")}catch{}function Rp({basename:e,children:t,window:a}){let n=W.useRef();n.current==null&&(n.current=H0({window:a,v5Compat:!0}));let r=n.current,[s,i]=W.useState({action:r.action,location:r.location}),o=W.useCallback(u=>{W.startTransition(()=>i(u))},[i]);return W.useLayoutEffect(()=>r.listen(o),[r,o]),W.createElement(Sp,{basename:e,children:t,location:s.location,navigationType:s.action,navigator:r})}function fx({basename:e,children:t,history:a}){let[n,r]=W.useState({action:a.action,location:a.location}),s=W.useCallback(i=>{W.startTransition(()=>r(i))},[r]);return W.useLayoutEffect(()=>a.listen(s),[a,s]),W.createElement(Sp,{basename:e,children:t,location:n.location,navigationType:n.action,navigator:a})}fx.displayName="unstable_HistoryRouter";var px=/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i,$n=W.forwardRef(function({onClick:t,discover:a="render",prefetch:n="none",relative:r,reloadDocument:s,replace:i,state:o,target:u,to:c,preventScrollReset:d,viewTransition:f,...m},h){let{basename:b}=W.useContext(Kt),y=typeof c=="string"&&px.test(c),w,g=!1;if(typeof c=="string"&&y&&(w=c,mx))try{let M=new URL(window.location.href),O=c.startsWith("//")?new URL(M.protocol+c):new URL(c),U=Ha(O.pathname,b);O.origin===M.origin&&U!=null?c=U+O.search+O.hash:g=!0}catch{na(!1,`<Link to="${c}"> contains an invalid URL which will probably break when clicked - please update to a valid URL path.`)}let v=Z0(c,{relative:r}),[x,$,S]=rT(n,m),C=yx(c,{replace:i,state:o,target:u,preventScrollReset:d,relative:r,viewTransition:f});function _(M){t&&t(M),M.defaultPrevented||C(M)}let T=W.createElement("a",{...m,...S,href:w||v,onClick:g||s?t:_,ref:oT(h,$),target:u,"data-discover":!y&&a==="render"?"true":void 0});return x&&!y?W.createElement(W.Fragment,null,T,W.createElement(dx,{page:v})):T});$n.displayName="Link";var Qa=W.forwardRef(function({"aria-current":t="page",caseSensitive:a=!1,className:n="",end:r=!1,style:s,to:i,viewTransition:o,children:u,...c},d){let f=Gs(i,{relative:c.relative}),m=Pe(),h=W.useContext(Qs),{navigator:b,basename:y}=W.useContext(Kt),w=h!=null&&wx(f)&&o===!0,g=b.encodeLocation?b.encodeLocation(f).pathname:f.pathname,v=m.pathname,x=h&&h.navigation&&h.navigation.location?h.navigation.location.pathname:null;a||(v=v.toLowerCase(),x=x?x.toLowerCase():null,g=g.toLowerCase()),x&&y&&(x=Ha(x,y)||x);let $=g!=="/"&&g.endsWith("/")?g.length-1:g.length,S=v===g||!r&&v.startsWith(g)&&v.charAt($)==="/",C=x!=null&&(x===g||!r&&x.startsWith(g)&&x.charAt(g.length)==="/"),_={isActive:S,isPending:C,isTransitioning:w},T=S?t:void 0,M;typeof n=="function"?M=n(_):M=[n,S?"active":null,C?"pending":null,w?"transitioning":null].filter(Boolean).join(" ");let O=typeof s=="function"?s(_):s;return W.createElement($n,{...c,"aria-current":T,className:M,ref:d,style:O,to:i,viewTransition:o},typeof u=="function"?u(_):u)});Qa.displayName="NavLink";var hx=W.forwardRef(({discover:e="render",fetcherKey:t,navigate:a,reloadDocument:n,replace:r,state:s,method:i=oc,action:o,onSubmit:u,relative:c,preventScrollReset:d,viewTransition:f,...m},h)=>{let b=bx(),y=xx(o,{relative:c}),w=i.toLowerCase()==="get"?"get":"post",g=typeof o=="string"&&px.test(o);return W.createElement("form",{ref:h,method:w,action:y,onSubmit:n?u:x=>{if(u&&u(x),x.defaultPrevented)return;x.preventDefault();let $=x.nativeEvent.submitter,S=$?.getAttribute("formmethod")||i;b($||x.currentTarget,{fetcherKey:t,method:S,navigate:a,replace:r,state:s,relative:c,preventScrollReset:d,viewTransition:f})},...m,"data-discover":!g&&e==="render"?"true":void 0})});hx.displayName="Form";function vx({getKey:e,storageKey:t,...a}){let n=W.useContext(qo),{basename:r}=W.useContext(Kt),s=Pe(),i=$p();$x({getKey:e,storageKey:t});let o=W.useMemo(()=>{if(!n||!e)return null;let c=mp(s,i,r,e);return c!==s.key?c:null},[]);if(!n||n.isSpaMode)return null;let u=((c,d)=>{if(!window.history.state||!window.history.state.key){let f=Math.random().toString(32).slice(2);window.history.replaceState({key:f},"")}try{let m=JSON.parse(sessionStorage.getItem(c)||"{}")[d||window.history.state.key];typeof m=="number"&&window.scrollTo(0,m)}catch(f){console.error(f),sessionStorage.removeItem(c)}}).toString();return W.createElement("script",{...a,suppressHydrationWarning:!0,dangerouslySetInnerHTML:{__html:`(${u})(${JSON.stringify(t||dp)}, ${JSON.stringify(o)})`}})}vx.displayName="ScrollRestoration";function gx(e){return`${e} must be used within a data router.  See https://reactrouter.com/en/main/routers/picking-a-router.`}function Cp(e){let t=W.useContext(Pr);return Re(t,gx(e)),t}function lT(e){let t=W.useContext(Qs);return Re(t,gx(e)),t}function yx(e,{target:t,replace:a,state:n,preventScrollReset:r,relative:s,viewTransition:i}={}){let o=fe(),u=Pe(),c=Gs(e,{relative:s});return W.useCallback(d=>{if(B3(d,t)){d.preventDefault();let f=a!==void 0?a:Hs(u)===Hs(c);o(e,{replace:f,state:n,preventScrollReset:r,relative:s,viewTransition:i})}},[u,o,c,a,n,t,e,r,s,i])}var uT=0,cT=()=>`__${String(++uT)}__`;function bx(){let{router:e}=Cp("useSubmit"),{basename:t}=W.useContext(Kt),a=M3();return W.useCallback(async(n,r={})=>{let{action:s,method:i,encType:o,formData:u,body:c}=I3(n,t);if(r.navigate===!1){let d=r.fetcherKey||cT();await e.fetch(d,a,r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,flushSync:r.flushSync})}else await e.navigate(r.action||s,{preventScrollReset:r.preventScrollReset,formData:u,body:c,formMethod:r.method||i,formEncType:r.encType||o,replace:r.replace,state:r.state,fromRouteId:a,flushSync:r.flushSync,viewTransition:r.viewTransition})},[e,t,a])}function xx(e,{relative:t}={}){let{basename:a}=W.useContext(Kt),n=W.useContext(sa);Re(n,"useFormAction must be used inside a RouteContext");let[r]=n.matches.slice(-1),s={...Gs(e||".",{relative:t})},i=Pe();if(e==null){s.search=i.search;let o=new URLSearchParams(s.search),u=o.getAll("index");if(u.some(d=>d==="")){o.delete("index"),u.filter(f=>f).forEach(f=>o.append("index",f));let d=o.toString();s.search=d?`?${d}`:""}}return(!e||e===".")&&r.route.index&&(s.search=s.search?s.search.replace(/^\?/,"?index&"):"?index"),a!=="/"&&(s.pathname=s.pathname==="/"?a:xn([a,s.pathname])),Hs(s)}var dp="react-router-scroll-positions",ic={};function mp(e,t,a,n){let r=null;return n&&(a!=="/"?r=n({...e,pathname:Ha(e.pathname,a)||e.pathname},t):r=n(e,t)),r==null&&(r=e.key),r}function $x({getKey:e,storageKey:t}={}){let{router:a}=Cp("useScrollRestoration"),{restoreScrollPosition:n,preventScrollReset:r}=lT("useScrollRestoration"),{basename:s}=W.useContext(Kt),i=Pe(),o=$p(),u=sx();W.useEffect(()=>(window.history.scrollRestoration="manual",()=>{window.history.scrollRestoration="auto"}),[]),dT(W.useCallback(()=>{if(u.state==="idle"){let c=mp(i,o,s,e);ic[c]=window.scrollY}try{sessionStorage.setItem(t||dp,JSON.stringify(ic))}catch(c){na(!1,`Failed to save scroll positions in sessionStorage, <ScrollRestoration /> will not work properly (${c}).`)}window.history.scrollRestoration="auto"},[u.state,e,s,i,o,t])),typeof document<"u"&&(W.useLayoutEffect(()=>{try{let c=sessionStorage.getItem(t||dp);c&&(ic=JSON.parse(c))}catch{}},[t]),W.useLayoutEffect(()=>{let c=a?.enableScrollRestoration(ic,()=>window.scrollY,e?(d,f)=>mp(d,f,s,e):void 0);return()=>c&&c()},[a,s,e]),W.useLayoutEffect(()=>{if(n!==!1){if(typeof n=="number"){window.scrollTo(0,n);return}try{if(i.hash){let c=document.getElementById(decodeURIComponent(i.hash.slice(1)));if(c){c.scrollIntoView();return}}}catch{na(!1,`"${i.hash.slice(1)}" is not a decodable element ID. The view will not scroll to it.`)}r!==!0&&window.scrollTo(0,0)}},[i,n,r]))}function dT(e,t){let{capture:a}=t||{};W.useEffect(()=>{let n=a!=null?{capture:a}:void 0;return window.addEventListener("pagehide",e,n),()=>{window.removeEventListener("pagehide",e,n)}},[e,a])}function wx(e,{relative:t}={}){let a=W.useContext(vp);Re(a!=null,"`useViewTransitionState` must be used within `react-router-dom`'s `RouterProvider`.  Did you accidentally import `RouterProvider` from `react-router`?");let{basename:n}=Cp("useViewTransitionState"),r=Gs(e,{relative:t});if(!a.isTransitioning)return!1;let s=Ha(a.currentLocation.pathname,n)||a.currentLocation.pathname,i=Ha(a.nextLocation.pathname,n)||a.nextLocation.pathname;return zo(r.pathname,i)!=null||zo(r.pathname,s)!=null}var At=new Rd({defaultOptions:{queries:{refetchOnWindowFocus:!1,retry:1,staleTime:1e4}}});var Ep="ironclaw_token",Ke="/api/webchat/v2",jr=class extends Error{constructor(t,{status:a,statusText:n,body:r,headers:s,payload:i}={}){super(t),this.name="ApiError",this.status=a,this.statusText=n,this.body=r,this.headers=s,this.payload=i}};function Sa(){return sessionStorage.getItem(Ep)||""}function Ys(e){e?sessionStorage.setItem(Ep,e):sessionStorage.removeItem(Ep)}function dc(){if(typeof crypto<"u"&&typeof crypto.randomUUID=="function")return crypto.randomUUID();let e=new Uint8Array(16);return(crypto?.getRandomValues||(t=>t))(e),Array.from(e,t=>t.toString(16).padStart(2,"0")).join("")}async function _x(e){let t=await e.text().catch(()=>"");if(!t)return{text:"",payload:void 0};if(!(e.headers.get("content-type")||"").includes("application/json"))return{text:t,payload:void 0};try{return{text:t,payload:JSON.parse(t)}}catch{return{text:t,payload:void 0}}}function Nx(e){return String(e).replace(/[_-]+/g," ").trim().replace(/^\w/,t=>t.toUpperCase())}function kx({payload:e,body:t,statusText:a}={}){if(e&&typeof e=="object"){if(e.validation_code){let s=Nx(e.validation_code);return e.field?`${s} (${e.field})`:s}let r=e.kind||e.error;if(r){let s=Nx(r);return e.field?`${s} (${e.field})`:s}}let n=(t||"").trim();return n&&n.length<=200&&!n.startsWith("{")&&!n.startsWith("[")?n:a||"Request failed"}async function H(e,t={}){let a=Sa(),n=new Headers(t.headers||{});n.set("Accept","application/json"),t.body&&!n.has("Content-Type")&&n.set("Content-Type","application/json"),a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(e,{credentials:"same-origin",...t,headers:n});if(!r.ok){let{text:i,payload:o}=await _x(r);throw new jr(kx({payload:o,body:i,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:i,headers:r.headers,payload:o})}return(r.headers.get("content-type")||"").includes("application/json")?r.json():r.text()}function mc(){return H(`${Ke}/session`)}function fc({clientActionId:e,requestedThreadId:t,projectId:a}={}){let n={client_action_id:e||dc()};return t&&(n.requested_thread_id=t),a&&(n.project_id=a),H(`${Ke}/threads`,{method:"POST",body:JSON.stringify(n)})}function Rx({limit:e,cursor:t}={}){let a=new URL(`${Ke}/threads`,window.location.origin);return e!=null&&a.searchParams.set("limit",String(e)),t&&a.searchParams.set("cursor",t),H(a.pathname+a.search)}function Cx({threadId:e}={}){return e?H(`${Ke}/threads/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("threadId is required"))}function Tp(e){return`${Ke}/threads/${encodeURIComponent(e)}/files`}function Ex({threadId:e,path:t}={}){if(!e)return Promise.reject(new Error("threadId is required"));let a=new URL(Tp(e),window.location.origin);return t&&a.searchParams.set("path",t),H(a.pathname+a.search)}function Tx({threadId:e,path:t}={}){if(!e||!t)return Promise.reject(new Error("threadId and path are required"));let a=new URL(`${Tp(e)}/stat`,window.location.origin);return a.searchParams.set("path",t),H(a.pathname+a.search)}function pc({threadId:e,path:t}={}){if(!e||!t)throw new Error("projectFileContentUrl requires threadId and path");let a=new URL(`${Tp(e)}/content`,window.location.origin);return a.searchParams.set("path",t),a.pathname+a.search}function Ax({limit:e,runLimit:t,includeCompleted:a}={}){let n=new URLSearchParams;e!=null&&n.set("limit",String(e)),t!=null&&n.set("run_limit",String(t)),a===!0&&n.set("include_completed","true");let r=n.toString();return H(`${Ke}/automations${r?`?${r}`:""}`)}function Dx({automationId:e}={}){return e?H(`${Ke}/automations/${encodeURIComponent(e)}/pause`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Mx({automationId:e}={}){return e?H(`${Ke}/automations/${encodeURIComponent(e)}/resume`,{method:"POST"}):Promise.reject(new Error("automationId is required"))}function Ox({automationId:e}={}){return e?H(`${Ke}/automations/${encodeURIComponent(e)}`,{method:"DELETE"}):Promise.reject(new Error("automationId is required"))}var Lx=`${Ke}/projects`;function mT(e){return`${Lx}/${encodeURIComponent(e)}`}function Px({limit:e}={}){let t=new URL(Lx,window.location.origin);return e!=null&&t.searchParams.set("limit",String(e)),H(t.pathname+t.search)}function Ux({projectId:e}={}){return e?H(mT(e)):Promise.reject(new Error("projectId is required"))}function jx(){return H(`${Ke}/outbound/preferences`)}function Fx(){return H(`${Ke}/outbound/targets`)}function Bx({finalReplyTargetId:e}={}){return H(`${Ke}/outbound/preferences`,{method:"POST",body:JSON.stringify({final_reply_target_id:e??null})})}function Ap({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:f}={}){let m=new URL(`${Ke}/logs`,window.location.origin);return e!=null&&m.searchParams.set("limit",String(e)),t&&m.searchParams.set("cursor",t),a&&m.searchParams.set("level",a),n&&m.searchParams.set("target",n),r&&m.searchParams.set("thread_id",r),s&&m.searchParams.set("run_id",s),i&&m.searchParams.set("turn_id",i),o&&m.searchParams.set("tool_call_id",o),u&&m.searchParams.set("tool_name",u),c&&m.searchParams.set("source",c),d&&m.searchParams.set("tail","true"),f&&m.searchParams.set("follow","true"),H(m.pathname+m.search)}function zx({limit:e,cursor:t,level:a,target:n,threadId:r,runId:s,turnId:i,toolCallId:o,toolName:u,source:c,tail:d,follow:f}={}){let m=new URL(`${Ke}/operator/logs`,window.location.origin);return e!=null&&m.searchParams.set("limit",String(e)),t&&m.searchParams.set("cursor",t),a&&m.searchParams.set("level",a),n&&m.searchParams.set("target",n),r&&m.searchParams.set("thread_id",r),s&&m.searchParams.set("run_id",s),i&&m.searchParams.set("turn_id",i),o&&m.searchParams.set("tool_call_id",o),u&&m.searchParams.set("tool_name",u),c&&m.searchParams.set("source",c),d&&m.searchParams.set("tail","true"),f&&m.searchParams.set("follow","true"),H(m.pathname+m.search)}function qx({threadId:e,content:t,attachments:a=[],clientActionId:n}){let r={client_action_id:n||dc(),content:t};return a.length>0&&(r.attachments=a),H(`${Ke}/threads/${encodeURIComponent(e)}/messages`,{method:"POST",body:JSON.stringify(r)})}function Ix({threadId:e,limit:t,cursor:a}={}){let n=new URL(`${Ke}/threads/${encodeURIComponent(e)}/timeline`,window.location.origin);return t!=null&&n.searchParams.set("limit",String(t)),a&&n.searchParams.set("cursor",a),H(n.pathname+n.search)}function Kx({threadId:e,messageId:t,attachmentId:a}={}){if(!e||!t||!a)throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");return`${Ke}/threads/${encodeURIComponent(e)}/messages/${encodeURIComponent(t)}/attachments/${encodeURIComponent(a)}`}async function Ca(e){let t=new URL(e,window.location.origin);if(t.origin!==window.location.origin)throw new jr("Invalid attachment URL.",{status:400,statusText:"Bad Request"});let a=Sa(),n=new Headers;a&&n.set("Authorization",`Bearer ${a}`);let r=await fetch(t.pathname+t.search,{credentials:"same-origin",headers:n});if(!r.ok){let{text:s,payload:i}=await _x(r);throw new jr(kx({payload:i,body:s,statusText:r.statusText}),{status:r.status,statusText:r.statusText,body:s,payload:i})}return await r.blob()}function Dp(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>t(n.result),n.onerror=()=>a(n.error||new Error("attachment read failed")),n.readAsDataURL(e)})}async function hc(e){return Dp(await Ca(e))}function Hx({threadId:e,afterCursor:t}={}){let a=new URL(`${Ke}/threads/${encodeURIComponent(e)}/events`,window.location.origin),n=Sa();return n&&a.searchParams.set("token",n),t&&a.searchParams.set("after_cursor",t),new EventSource(a.toString())}function Qx({threadId:e,runId:t,reason:a,clientActionId:n}={}){let r={client_action_id:n||dc()};return a&&(r.reason=a),H(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/cancel`,{method:"POST",body:JSON.stringify(r)})}function Mp({threadId:e,runId:t,gateRef:a,resolution:n,always:r,credentialRef:s,clientActionId:i,signal:o}={}){let u={client_action_id:i||dc(),resolution:n};return r!=null&&(u.always=r),s&&(u.credential_ref=s),H(`${Ke}/threads/${encodeURIComponent(e)}/runs/${encodeURIComponent(t)}/gates/${encodeURIComponent(a)}/resolve`,{method:"POST",signal:o,body:JSON.stringify(u)})}function Vx({provider:e,accountLabel:t,token:a,threadId:n,runId:r,gateRef:s,signal:i}={}){return H("/api/reborn/product-auth/manual-token/submit",{method:"POST",signal:i,body:JSON.stringify({provider:e,account_label:t,token:a,thread_id:n,run_id:r,gate_ref:s})})}function Gx(e,{action:t,payload:a}={}){let n={};return t&&(n.action=t),a!==void 0&&(n.payload=a),H(`${Ke}/extensions/${encodeURIComponent(e)}/setup`,{method:"POST",body:JSON.stringify(n)})}function Js(){return Promise.resolve({engine_v2_enabled:!1,restart_enabled:!1,total_connections:null,llm_backend:null,llm_model:null,todo:!0})}async function Yx(){try{let e=await fetch("/auth/providers",{headers:{Accept:"application/json"},credentials:"same-origin"});if(!e.ok)return{providers:[]};let t=await e.json();return{providers:Array.isArray(t?.providers)?t.providers:[]}}catch{return{providers:[]}}}async function Jx(e){let t=await fetch("/auth/session/exchange",{method:"POST",headers:{Accept:"application/json","Content-Type":"application/json"},credentials:"same-origin",body:JSON.stringify({ticket:e})});if(!t.ok)throw new jr("Could not complete sign-in.",{status:t.status,statusText:t.statusText,headers:t.headers});let a=await t.json(),n=(a?.token||"").trim();if(!n)throw new jr("Sign-in response did not include a token.",{status:t.status,statusText:t.statusText,headers:t.headers,payload:a});return n}async function Xx(){let e=Sa();if(!e)return;let t=new Headers({Accept:"application/json"});t.set("Authorization",`Bearer ${e}`);try{await fetch("/auth/logout",{method:"POST",headers:t,credentials:"same-origin"})}catch{}}var vc="anon",Zx=vc;function Wx(e){Zx=e&&e.tenant_id&&e.user_id?`${e.tenant_id}:${e.user_id}`:vc}function kt(){return Zx}var e$="ironclaw:v2-thread-pins:",Op=new Set,wn=new Set,Lp=null;function Pp(){return`${e$}${kt()}`}function fT(){try{let e=window.localStorage.getItem(Pp());if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>typeof a=="string"):[]}catch{return[]}}function pT(){try{wn.size===0?window.localStorage.removeItem(Pp()):window.localStorage.setItem(Pp(),JSON.stringify([...wn]))}catch{}}function t$(){let e=kt();if(e!==Lp){wn.clear();for(let t of fT())wn.add(t);Lp=e}}function a$(){return new Set(wn)}function n$(){let e=a$();for(let t of Op)try{t(e)}catch{}}function r$(e){e&&(t$(),wn.has(e)?wn.delete(e):wn.add(e),pT(),n$())}function s$(){return t$(),a$()}function i$(e){return Op.add(e),()=>{Op.delete(e)}}function o$(){wn.clear(),Lp=kt();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith(e$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}n$()}var hT=0,Fr={accept:[],maxCount:10,maxFileBytes:5*1024*1024,maxTotalBytes:10*1024*1024};function Up(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":"document"}function l$(e){let t=(e||"").toLowerCase();return t.startsWith("image/")?"image":t.startsWith("audio/")?"audio":t.startsWith("video/")?"video":t==="application/pdf"?"pdf":vT(t)?"text":"download"}function vT(e){return e.startsWith("text/")||e==="application/json"||e==="application/xml"||e==="application/csv"||e.endsWith("+json")||e.endsWith("+xml")}function Io(e){if(!Number.isFinite(e)||e<0)return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a>=10||Number.isInteger(a)?Math.round(a):Math.round(a*10)/10} ${t[n]}`}function gT(e,t){if(!t||t.length===0)return!0;let a=(e.type||"").toLowerCase(),n=(e.name||"").toLowerCase();return t.some(r=>{let s=r.trim().toLowerCase();return s?s==="*/*"||s==="*"?!0:s.endsWith("/*")?a.startsWith(s.slice(0,-1)):s.startsWith(".")?n.endsWith(s):a===s:!1})}function yT(e){return new Promise((t,a)=>{let n=new FileReader;n.onload=()=>{typeof n.result=="string"?t(n.result):a(new Error("file read produced no data URL"))},n.onerror=()=>a(n.error||new Error("file read failed")),n.readAsDataURL(e)})}function bT(e,t){let a=e.indexOf(",");if(a<0)return{mime:t||"",base64:""};let n=e.slice(0,a),r=e.slice(a+1),s=n.match(/^data:([^;]*)/);return{mime:s&&s[1]||t||"",base64:r}}async function u$(e,{limits:t,existing:a=[],t:n}){let r=t||Fr,s=[],i=[],o=a.length,u=a.reduce((c,d)=>c+(d.sizeBytes||0),0);for(let c of e){if(o>=r.maxCount){i.push(n("chat.attachmentTooMany",{max:r.maxCount}));break}if(!gT(c,r.accept)){i.push(n("chat.attachmentUnsupportedType",{name:c.name||"file"}));continue}if(c.size>r.maxFileBytes){i.push(n("chat.attachmentTooLarge",{name:c.name||"file",max:Io(r.maxFileBytes)}));continue}if(u+c.size>r.maxTotalBytes){let y=n("chat.attachmentTotalTooLarge",{max:Io(r.maxTotalBytes)});i.includes(y)||i.push(y);continue}let d;try{d=await yT(c)}catch{i.push(n("chat.attachmentReadFailed",{name:c.name||"file"}));continue}let{mime:f,base64:m}=bT(d,c.type),h=f||"application/octet-stream",b=Up(h);s.push({id:`staged-${hT++}`,filename:c.name||"attachment",mimeType:h,kind:b,sizeBytes:c.size,sizeLabel:Io(c.size),dataBase64:m,previewUrl:b==="image"?d:null}),o+=1,u+=c.size}return{staged:s,errors:i}}function c$(e){return{mime_type:e.mimeType,filename:e.filename,data_base64:e.dataBase64}}function d$(e){return{id:e.id,filename:e.filename,mime_type:e.mimeType,kind:e.kind,size_label:e.sizeLabel,preview_url:e.previewUrl}}function xT(e,t){let a=e.attachments;if(!(!Array.isArray(a)||a.length===0))return a.map(n=>{let r=n.kind||Up(n.mime_type),s=t&&n.storage_key&&e.message_id&&n.id?Kx({threadId:t,messageId:e.message_id,attachmentId:n.id}):null;return{id:n.id,filename:n.filename||"attachment",mime_type:n.mime_type||"",kind:r,size_label:Number.isFinite(n.size_bytes)?Io(n.size_bytes):"",preview_url:null,fetch_url:s}})}function f$(e,t=[],a=null){let n=new Set,r=[];for(let s of e||[]){if(s.kind==="tool_result_reference")continue;if(s.kind==="capability_display_preview"){let c=NT(s);if(!c)continue;let d=`tool-${c.invocationId}`;if(n.has(d))continue;n.add(d),r.push({id:d,role:"tool_activity",...c,timestamp:m$(s)||c.updatedAt||null,sequence:s.sequence,activityOrder:c.activityOrder,activityOrderSource:c.activityOrderSource,turnRunId:s.turn_run_id||null});continue}let i=`msg-${s.message_id}`;if(n.has(i))continue;n.add(i);let o=ST(s),u=o==="user"&&(s.status==="rejected_busy"||s.status==="deferred_busy");r.push({id:i,role:o,content:s.content||"",attachments:xT(s,a),timestamp:m$(s),kind:s.kind,status:u?"error":s.status,...u&&{error:"This message wasn't sent because Ironclaw was busy. Resend it to try again."},isFinalReply:wT(s),sequence:s.sequence,turnRunId:s.turn_run_id||null})}for(let s of t){if(n.has(s.id))continue;let i=$T(s);i.timelineMessageId&&n.has(`msg-${i.timelineMessageId}`)||r.push(i)}return r}function $T(e){return{...e,role:e.role||"user",isOptimistic:e.isOptimistic!==!1}}function wT(e){return(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized"}function ST(e){switch(e.kind){case"user":case"user_message":return"user";case"assistant":case"assistant_message":case"tool_result":return"assistant";case"system":return"system";default:return e.actor_id?"user":"assistant"}}function m$(e){return e.received_at||e.created_at||null}function NT(e){if(!e.content)return null;let t;try{t=JSON.parse(e.content)}catch(a){return console.warn("Failed to parse capability_display_preview envelope",a),null}return!t||!t.invocation_id?null:jp(t)}var _T="gate_declined";function jp(e){let t=e.status==="failed"||e.status==="killed",a=e.error_kind||null,n=v$(e.activity_order);return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Ho(e.title||e.capability_id)||"tool",toolStatus:h$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:t?null:e.output_preview||e.output_summary||null,toolError:t&&(e.output_summary||e.output_preview||e.result_ref||p$(a))||null,toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:e.result_ref||null,truncated:!!e.truncated,outputBytes:e.output_bytes??null,outputKind:e.output_kind||null,turnRunId:e.turn_run_id||null,activityOrder:n,activityOrderSource:Number.isFinite(n)?"projection":null}}function Fp(e){let t=v$(e.activity_order),a=e.error_kind||null;return{invocationId:e.invocation_id,callId:e.invocation_id,capabilityId:e.capability_id||null,toolName:Ho(e.capability_id)||"tool",toolStatus:h$(e.status,a),toolDetail:e.subtitle||null,toolParameters:e.input_summary||null,toolResultPreview:null,toolError:p$(a),toolErrorKind:a,toolDurationMs:null,updatedAt:e.updated_at||null,resultRef:null,truncated:!1,outputBytes:e.output_bytes??null,outputKind:null,turnRunId:e.turn_run_id||null,activityOrder:t,activityOrderSource:Number.isFinite(t)?"projection":null}}function p$(e){let t=typeof e=="string"?e.trim():"";if(!t)return null;switch(t.toLowerCase().replaceAll("-","_")){case"backend":return"The tool backend failed.";case"security":case"security_rejected":case"security_rejection":return"The tool response was blocked by a security check.";case"cancelled":return"The tool call was cancelled.";case"timeout":return"The tool call timed out.";case"invalid_request":return"The tool request was invalid.";case"auth":case"authentication":case"authorization":return"The tool needs authentication.";case"permission":case"approval_denied":return"The tool call was not allowed.";default:return t.replaceAll("_"," ")}}function Ko(e){return e==="success"||e==="error"||e==="declined"}function Ho(e){let t=typeof e=="string"?e.trim():"";if(!t)return"";let a=t.split(".");return a[a.length-1]||t}function h$(e,t=null){if(t===_T)return"declined";switch(e){case"completed":return"success";case"failed":case"killed":return"error";case"started":case"running":default:return"running"}}function v$(e){let t=Number(e);return Number.isFinite(t)?t:null}var kT=50,Va=new Map,RT=30;function Qo(e,t){for(Va.delete(e),Va.set(e,t);Va.size>RT;){let a=Va.keys().next().value;Va.delete(a)}}function Vo(e){return`${kt()}:${e}`}function b$(){Va.clear()}function x$(e,t={}){let{getPendingMessages:a,setPendingMessages:n}=t,r=e?Va.get(Vo(e)):null,[s,i]=p.default.useState({messages:r?.messages||[],nextCursor:r?.nextCursor||null,isLoading:!1,loadError:null}),o=p.default.useRef(new Set),u=p.default.useRef(e);u.current=e;let c=p.default.useCallback(async(f,m={})=>{let{preserveClientOnly:h=!1,finalReplyTimestampByRun:b=null}=m;if(!e){i({messages:[],nextCursor:null,isLoading:!1,loadError:null});return}if(o.current.has(e))return;o.current.add(e);let y=kt(),w=Vo(e);i(g=>({...g,isLoading:!0}));try{let g=await Ix({threadId:e,limit:kT,cursor:f});if(kt()!==y)return;let v=f?[]:a?.()||[],x=f$(g.messages||[],v,e),$=g.next_cursor||null;if(f||n?.([]),!f){let S=Va.get(w)?.messages||[],C=g$(x,S,{preserveClientOnly:h,finalReplyTimestampByRun:b});Qo(w,{messages:C,nextCursor:$})}i(S=>{if(u.current!==e)return S;let C;return f?C=CT(x,S.messages):C=g$(x,S.messages,{preserveClientOnly:h,finalReplyTimestampByRun:b}),Qo(w,{messages:C,nextCursor:$}),{messages:C,nextCursor:$,isLoading:!1,loadError:null}})}catch(g){if(console.error("Failed to load timeline:",g),kt()!==y)return;i(v=>u.current===e?{...v,isLoading:!1,loadError:"Failed to load conversation history."}:v)}finally{o.current.delete(e)}},[e,a,n]);p.default.useEffect(()=>{let f=e?Va.get(Vo(e)):null;i({messages:f?.messages||[],nextCursor:f?.nextCursor||null,isLoading:!!e&&!f,loadError:null}),e&&c()},[e,c]);let d=p.default.useCallback((f,m)=>{if(!f)return;let h=Vo(f),b=g=>typeof m=="function"?m(g||[]):m;if(u.current===f){i(g=>{let v=b(g.messages||[]);return Qo(h,{messages:v,nextCursor:g.nextCursor||null}),{...g,messages:v}});return}let y=Va.get(h)||{messages:[],nextCursor:null},w=b(y.messages||[]);Qo(h,{messages:w,nextCursor:y.nextCursor||null})},[]);return{messages:s.messages,hasMore:!!s.nextCursor,nextCursor:s.nextCursor,isLoading:s.isLoading,loadError:s.loadError,loadHistory:c,seedThreadMessages:d,setMessages:f=>i(m=>{let h=typeof f=="function"?f(m.messages):f;return e&&Qo(Vo(e),{messages:h,nextCursor:m.nextCursor}),{...m,messages:h}})}}function CT(e,t){let a=new Set(t.map(n=>n?.id).filter(Boolean));return[...e.filter(n=>!a.has(n?.id)),...t]}function g$(e,t,a={}){let{preserveClientOnly:n=!1,finalReplyTimestampByRun:r=null}=a,s=MT(e,t,{finalReplyTimestampByRun:r}),i=new Set(s.map(u=>u?.id).filter(Boolean)),o=t.filter(u=>!u||typeof u.id!="string"||i.has(u.id)?!1:LT(u)?!0:typeof u.timelineMessageId=="string"&&i.has(`msg-${u.timelineMessageId}`)?!1:DT(u)?!0:n&&u.id.startsWith("err-"));return o.length>0?ET(s,o):s}function ET(e,t){let a=[...e];for(let n of t){let r=TT(a,n);r>=0?a.splice(r+1,0,n):a.push(n)}return a}function TT(e,t){if(!AT(t))return-1;let a=typeof t.turnRunId=="string"&&t.turnRunId.trim()?t.turnRunId:null;if(!a)return-1;for(let n=e.length-1;n>=0;n-=1)if(e[n]?.turnRunId===a)return n;return-1}function AT(e){return e?.role==="error"&&typeof e.id=="string"&&e.id.startsWith("err-")}function DT(e){return e?.isOptimistic===!0&&typeof e.id=="string"&&e.id.startsWith("pending-")&&(e.role==="user"||e.role==="assistant")}function MT(e,t,a={}){let{finalReplyTimestampByRun:n=null}=a,r=new Map,s=new Map;for(let i of t||[])i&&(typeof i.id=="string"&&r.set(i.id,i),typeof i.timelineMessageId=="string"&&r.set(`msg-${i.timelineMessageId}`,i),Bp(i)&&typeof i.turnRunId=="string"&&s.set(i.turnRunId,i));return r.size===0&&s.size===0&&!n?e:e.map(i=>{if(!i||typeof i.id!="string")return i;let o=typeof i.turnRunId=="string"?i.turnRunId:null,u=r.get(i.id)||(Bp(i)&&o?s.get(o):null),c=Bp(i)&&o?n?.[o]:null,d=i.timestamp||u?.timestamp||c,f=OT(i,u);return d&&(f={...f,timestamp:d}),f})}function Bp(e){return e?.role==="assistant"&&e?.isFinalReply===!0}function OT(e,t){if(e?.role!=="tool_activity"||t?.role!=="tool_activity")return e;let a=e;for(let n of["toolDetail","toolParameters","toolResultPreview"])!y$(a[n])&&y$(t[n])&&(a={...a,[n]:t[n]});return a}function y$(e){return typeof e=="string"?e.trim().length>0:e!=null}function LT(e){return e?.role==="tool_activity"||e?.role==="thinking"}var Zs="__new__",$$="ironclaw:v2-draft:";function Xs(e){return`${$$}${kt()}:${e||Zs}`}function zp(e){try{return window.localStorage.getItem(Xs(e))||""}catch{return""}}function qp(e,t){try{t?window.localStorage.setItem(Xs(e),t):window.localStorage.removeItem(Xs(e))}catch{}}function w$(e){qp(e,"")}var Go=new Map;function Ip(e){return Go.get(Xs(e))||[]}function S$(e,t){let a=Xs(e);t&&t.length>0?Go.set(a,t):Go.delete(a)}function N$(e){Go.delete(Xs(e))}function _$(){Go.clear();try{let e=[];for(let t=0;t<window.localStorage.length;t+=1){let a=window.localStorage.key(t);a&&a.startsWith($$)&&e.push(a)}e.forEach(t=>window.localStorage.removeItem(t))}catch{}}function PT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{return new URLSearchParams(a).get(t)||""}catch{return""}}function UT(e,t){if(!e)return"";let a=e.startsWith("#")?e.slice(1):e;try{let n=new URLSearchParams(a);n.delete(t);let r=n.toString();return r?`#${r}`:""}catch{return e}}function jT(){let e=new URL(window.location.href),t=(e.searchParams.get("token")||"").trim(),a=PT(e.hash,"token").trim(),n=a||t;if(!n)return"";t&&e.searchParams.delete("token");let r=a?UT(e.hash,"token"):e.hash;return window.history.replaceState({},"",e.pathname+e.search+r),Sa()?"":(Ys(n),n)}function FT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_ticket")||"").trim();return t?(e.searchParams.delete("login_ticket"),window.history.replaceState({},"",e.pathname+e.search+e.hash),t):""}var BT={denied:"Sign-in was cancelled.",invalid_state:"Your sign-in session expired. Please try again.",invalid_request:"Sign-in request was malformed. Please try again.",provider_mismatch:"Sign-in provider mismatch. Please try again.",unauthorized:"This account is not authorized.",exchange_failed:"Could not complete sign-in with the provider.",server_error:"Sign-in is temporarily unavailable."};function zT(){let e=new URL(window.location.href),t=(e.searchParams.get("login_error")||"").trim();return t?(e.searchParams.delete("login_error"),window.history.replaceState({},"",e.pathname+e.search+e.hash),BT[t]||"Could not complete sign-in. Please try again."):""}function k$(){let[e,t]=p.default.useState(()=>jT()||Sa()),[a,n]=p.default.useState(()=>zT()),[r]=p.default.useState(()=>FT()),[s,i]=p.default.useState(null),[o,u]=p.default.useState(()=>!!(r&&!Sa())),[c,d]=p.default.useState(()=>!!Sa());p.default.useEffect(()=>{if(!r||Sa()){u(!1);return}let b=!1;return Jx(r).then(y=>{b||(Ys(y),d(!0),t(y),i(null),n(""),u(!1),At.clear())}).catch(()=>{b||(n("Could not complete sign-in. Please try again."),u(!1))}),()=>{b=!0}},[r]),p.default.useEffect(()=>{if(!e||o){i(null),d(!1);return}let b=!1;return d(!0),mc().then(y=>{b||(i(y),d(!1))}).catch(y=>{b||(i(null),d(!1),(y?.status===401||y?.status===403)&&(Ys(""),t(""),n("Your session expired. Please sign in again."),At.clear()))}),()=>{b=!0}},[e,o]),Wx(s);let f=p.default.useRef(null);p.default.useEffect(()=>{let b=kt();f.current&&f.current!==vc&&f.current!==b&&(b$(),_$(),o$()),f.current=b},[s]);let m=p.default.useCallback(b=>{Ys(b),d(!!b),t(b),i(null),n(""),At.clear()},[]),h=p.default.useCallback(()=>{Xx().catch(()=>{}),Ys(""),d(!1),t(""),i(null),n(""),At.clear()},[]);return{token:e,profile:s?{tenant_id:s.tenant_id,user_id:s.user_id}:null,error:a,setError:n,isChecking:o||c,isAuthenticated:!!e,isAdmin:!!s?.capabilities?.operator_webui_config,rebornProjectsEnabled:!!s?.features?.reborn_projects,signIn:m,signOut:h}}var Br="/chat",Yo=[{id:"chat",path:"/chat",labelKey:"nav.chat"},{id:"workspace",path:"/workspace",labelKey:"nav.workspace"},{id:"projects",path:"/projects",labelKey:"nav.projects",hidden:!0},{id:"jobs",path:"/jobs",labelKey:"nav.jobs",hidden:!0},{id:"routines",path:"/routines",labelKey:"nav.routines",hidden:!0},{id:"automations",path:"/automations",labelKey:"nav.automations"},{id:"missions",path:"/missions",labelKey:"nav.missions",hidden:!0},{id:"extensions",path:"/extensions",labelKey:"nav.extensions"},{id:"logs",path:"/logs",labelKey:"nav.logs",hidden:!0},{id:"settings",path:"/settings",labelKey:"nav.settings",hidden:!1},{id:"admin",path:"/admin",labelKey:"nav.admin",hidden:!0}];var qT=[{id:"inference",labelKey:"settings.inference",icon:"spark"},{id:"tools",labelKey:"settings.tools",icon:"tool"},{id:"skills",labelKey:"settings.skills",icon:"file"},{id:"traces",labelKey:"settings.traceCommons",icon:"layers"},{id:"language",labelKey:"settings.language",icon:"globe"}],IT=[{id:"registry",labelKey:"extensions.registry",icon:"plus"},{id:"channels",labelKey:"extensions.channels",icon:"send"},{id:"mcp",labelKey:"extensions.mcp",icon:"pulse"}],KT=[{id:"dashboard",labelKey:"admin.tab.dashboard",icon:"pulse"},{id:"users",labelKey:"admin.tab.users",icon:"lock"},{id:"usage",labelKey:"admin.tab.usage",icon:"spark"}],gc={settings:qT,extensions:IT,admin:KT};var R$="ironclaw:v2-theme";function HT(){try{if(window.__IRONCLAW_INITIAL_THEME__==="light"||window.__IRONCLAW_INITIAL_THEME__==="dark")return window.__IRONCLAW_INITIAL_THEME__;let e=document.documentElement.dataset.theme;if(e==="light"||e==="dark")return e;let t=window.localStorage.getItem(R$);return t==="light"||t==="dark"?t:window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}catch{return"light"}}function yc(){let[e,t]=p.default.useState(HT);p.default.useEffect(()=>{document.documentElement.dataset.theme=e;try{window.localStorage.setItem(R$,e)}catch{}},[e]);let a=p.default.useCallback(()=>{t(n=>n==="dark"?"light":"dark")},[]);return{theme:e,toggleTheme:a}}function C$(e){return K({enabled:!!e,queryKey:["gateway-status",e],queryFn:Js,refetchInterval:3e4})}var QT="/api/webchat/v2/operator/config",bc="/api/webchat/v2/settings/tools",Ws="agent.auto_approve_tools",E$="tool.",VT=new Set(["always_allow","ask_each_time","disabled"]),GT=new Set(["default","always_allow","ask_each_time","disabled"]);function T$(e){return e==="ask"?"ask_each_time":VT.has(e)?e:"ask_each_time"}function YT(e){return e==="ask"?"ask_each_time":GT.has(e)?e:"default"}function JT(e){return["default","global","override"].includes(e)?e:"default"}function A$(e){if(!e?.key?.startsWith(E$))return null;let t=e.value||{};return{name:t.name||e.key.slice(E$.length),description:t.description||"",state:T$(t.state),default_state:T$(t.default_state),locked:!!(t.locked||e.mutable===!1),effective_source:JT(t.effective_source||e.source)}}function XT(e){let t={};for(let a of e.entries||[])a?.key===Ws&&(t[Ws]=!!a.value);return t}async function D$(){let e=await H(bc);return{settings:XT(e),diagnostics:e.diagnostics||[],precedence:e.precedence||[]}}async function Kp(e,t){if(e===Ws){let n=await H(bc,{method:"POST",body:JSON.stringify({enabled:!!t})});return{success:!0,entry:n.entry,value:n.entry?.value}}let a=await H(`${QT}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({value:t})});return{success:!0,entry:a.entry,value:a.entry?.value}}async function M$(e){let t=e?.settings||{},a=[];return Object.prototype.hasOwnProperty.call(t,Ws)&&a.push(await Kp(Ws,!!t[Ws])),{success:!0,imported:a.length,results:a}}function xc(){return H("/api/webchat/v2/llm/providers")}function O$(e){return H("/api/webchat/v2/llm/providers",{method:"POST",body:JSON.stringify(e)})}function L$(e){return H(`/api/webchat/v2/llm/providers/${encodeURIComponent(e)}/delete`,{method:"POST"})}function Jo(e){return H("/api/webchat/v2/llm/active",{method:"POST",body:JSON.stringify(e)})}function P$(e){return H("/api/webchat/v2/llm/test-connection",{method:"POST",body:JSON.stringify(e)})}function U$(e){return H("/api/webchat/v2/llm/list-models",{method:"POST",body:JSON.stringify(e)})}function j$(e){return H("/api/webchat/v2/llm/nearai/login",{method:"POST",body:JSON.stringify(e)})}function F$(e){return H("/api/webchat/v2/llm/nearai/wallet",{method:"POST",body:JSON.stringify(e)})}function B$(){return H("/api/webchat/v2/llm/codex/login",{method:"POST"})}async function z$(){let e=await H(bc);return{tools:(e.entries||[]).map(A$).filter(Boolean),diagnostics:e.diagnostics||[]}}async function q$(e,t){let a=YT(t),n=await H(`${bc}/${encodeURIComponent(e)}`,{method:"POST",body:JSON.stringify({state:a})});return{success:!0,tool:A$(n.entry),entry:n.entry}}function I$(){return H("/api/webchat/v2/extensions")}function K$(){return H("/api/webchat/v2/extensions/registry")}function H$(){return H("/api/webchat/v2/skills")}function Q$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`)}function V$(e){return H("/api/webchat/v2/skills/install",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(e)})}function G$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"PUT",headers:{"X-Confirm-Action":"true"},body:JSON.stringify(t)})}function Y$(e){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}`,{method:"DELETE",headers:{"X-Confirm-Action":"true"}})}function J$(e,t){return H(`/api/webchat/v2/skills/${encodeURIComponent(e)}/auto-activate`,{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:t})})}function X$(e){return H("/api/webchat/v2/skills/auto-activate-learned",{method:"POST",headers:{"X-Confirm-Action":"true"},body:JSON.stringify({enabled:e})})}function Z$(){return H("/api/webchat/v2/traces/credit")}function W$(e){return H(`/api/webchat/v2/traces/holds/${encodeURIComponent(e)}/authorize`,{method:"POST"})}function ew(){return Promise.resolve({users:[],todo:!0})}function tw(e){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}function aw(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 users endpoint"})}var Hp="\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022",Qp=[{value:"open_ai_completions",label:"OpenAI Compatible"},{value:"anthropic",label:"Anthropic"},{value:"ollama",label:"Ollama"},{value:"nearai",label:"NEAR AI"}];function Xo(e){return Qp.find(t=>t.value===e)?.label||e}function ei(e,t){return(e.builtin?t[e.id]||{}:{}).base_url||e.env_base_url||e.base_url||""}function nw(e,t,a,n){let r=e.builtin?t[e.id]||{}:{};return e.id===a?n||r.model||e.env_model||e.default_model||"":r.model||e.env_model||e.default_model||""}function $c(e,t){return(e.builtin?t[e.id]||{}:{}).model||e.env_model||e.default_model||""}function rw(e){return e?e.builtin?e.accepts_api_key!==void 0?e.accepts_api_key!==!1:e.api_key_required!==!1:e.adapter!=="ollama":!1}function zr(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Hp||typeof r=="string"&&r.length>0;return!n||e.has_api_key===!0||s?(e.builtin?e.base_url_required===!0:!0)?ei(e,t).trim().length>0:!0:!1}function ZT(e,t,a){return e.id===a?"active":zr(e,t)?"ready":"setup"}function sw(e,t,a){let n={active:[],ready:[],setup:[]};if(!Array.isArray(e))return n;for(let r of e){let s=ZT(r,t,a);n[s]&&n[s].push(r)}return n}function wc(e,t){let a=e.builtin?t[e.id]||{}:{},n=e.builtin?e.api_key_required!==!1:e.adapter!=="ollama",r=e.builtin?a.api_key:e.api_key,s=r===Hp||typeof r=="string"&&r.length>0;return n&&e.has_api_key!==!0&&!s?"api_key":(e.builtin?e.base_url_required===!0:!0)&&!ei(e,t).trim()?"base_url":"ok"}function Vp(e,t,a,n){let r=t.baseUrl.trim(),s=t.model.trim(),i={adapter:e?.builtin?e.adapter:t.adapter,base_url:r||e?.base_url||"",provider_id:e?.id||t.id.trim(),provider_type:e?.builtin?"builtin":"custom"};s&&(i.model=s),a.trim()&&(i.api_key=a.trim());let o=e?.builtin?n[e.id]||{}:{};return!i.api_key&&o.api_key===Hp&&(i.api_key=void 0),i}function iw(e){return e.toLowerCase().replace(/[^a-z0-9_]+/g,"-").replace(/^-|-$/g,"")}function ow(e){return/^[a-z0-9_-]+$/.test(e)}function lw(e,t){if(!Array.isArray(t)||t.length===0)return null;let a=(e||"").trim();return!a||!t.includes(a)?t[0]:null}var WT=Object.freeze({});function ti({settings:e,gatewayStatus:t,enabled:a=!0}){let n=J(),r=K({queryKey:["llm-providers"],queryFn:xc,enabled:a,staleTime:6e4}),s=a?r.data||{providers:[],active:null}:{providers:[],active:null},i=a&&r.isError,o=WT,u=(s.providers||[]).map($=>({...$,name:$.description,has_api_key:$.api_key_set===!0})),c=!!(s.active?.provider_id||t?.llm_backend),d=c?s.active?.provider_id||t?.llm_backend:null,f=d||"nearai",m=s.active?.model||t?.llm_model||"",h=u.filter($=>$.builtin),b=u.filter($=>!$.builtin),y=[...u].sort(($,S)=>$.id===d?-1:S.id===d?1:($.name||$.id).localeCompare(S.name||S.id)),w=()=>{n.invalidateQueries({queryKey:["llm-providers"]})},g=Q({mutationFn:async $=>{if(!zr($,o)){let C=wc($,o);throw new Error(C==="base_url"?"base_url":"api_key")}let S=$c($,o);if(!S)throw new Error("model");return await Jo({provider_id:$.id,model:S}),$},onSuccess:w}),v=Q({mutationFn:async({provider:$,form:S,apiKey:C,editingProvider:_})=>{let T=!!$?.builtin,O={id:(T?$.id:S.id.trim()).trim(),name:T?$.name||$.id:S.name.trim(),adapter:T?$.adapter:S.adapter,base_url:S.baseUrl.trim()||$?.base_url||"",default_model:S.model.trim()||void 0};return C.trim()&&(O.api_key=C.trim()),(_||$)?.id===f&&O.default_model&&(O.set_active=!0,O.model=O.default_model),await O$(O),O},onSuccess:w}),x=Q({mutationFn:async $=>(await L$($.id),$),onSuccess:w});return{providers:y,builtinProviders:h,customProviders:b,builtinOverrides:o,activeProviderId:d,selectedModel:m,hasActiveProvider:c,isError:i,isLoading:r.isLoading,error:r.error,setActiveProvider:$=>g.mutateAsync($),saveCustomProvider:$=>v.mutateAsync($),saveBuiltinProvider:$=>v.mutateAsync($),deleteCustomProvider:$=>x.mutateAsync($),testConnection:P$,listModels:U$,isBusy:g.isPending||v.isPending||x.isPending}}function uw({isLoading:e,hasActiveProvider:t,isError:a}){return!e&&!t&&!a}var cw="ironclaw:v2-sidebar-open";function dw(){return typeof window>"u"?null:window}function mw(){try{return dw()?.localStorage||null}catch{return null}}function fw(e=mw()){try{return e?.getItem(cw)!=="false"}catch{return!0}}function pw(e,t=mw()){try{t?.setItem(cw,e?"true":"false")}catch{}}function hw(e=dw()){try{return e?.matchMedia?.("(min-width: 768px)").matches===!0}catch{return!1}}function vw(e,t){return t?{...e,desktopOpen:!e.desktopOpen}:{...e,mobileOpen:!e.mobileOpen}}function gw(e,t){return t?e.desktopOpen:e.mobileOpen}function yw({onNewChat:e}={}){let t=fe(),[a,n]=p.default.useState(()=>({mobileOpen:!1,desktopOpen:fw()})),[r,s]=p.default.useState(()=>hw());p.default.useEffect(()=>{let d=window.matchMedia("(min-width: 768px)"),f=()=>s(d.matches);return f(),d.addEventListener?.("change",f),()=>d.removeEventListener?.("change",f)},[]),p.default.useEffect(()=>{pw(a.desktopOpen)},[a.desktopOpen]);let i=p.default.useCallback(()=>{n(d=>({...d,mobileOpen:!1}))},[]),o=p.default.useCallback(()=>{n(d=>vw(d,r))},[r]),u=p.default.useCallback(async()=>{let d=await e?.(),f=typeof d=="string"&&d.length>0?d:null;t(f?`/chat/${f}`:"/chat"),i()},[t,i,e]),c=p.default.useCallback(d=>{t(`/chat/${d}`),i()},[t,i]);return{mobileOpen:a.mobileOpen,desktopOpen:a.desktopOpen,currentOpen:gw(a,r),close:i,toggle:o,newChat:u,selectThread:c}}var Gp=new Set,eA=0;function ai(e,t={}){let a={id:++eA,message:e,tone:t.tone||"info",duration:t.duration??2600};return Gp.forEach(n=>n(a)),a.id}function bw(e){return Gp.add(e),()=>Gp.delete(e)}function tA(e){return e?.status===409&&e?.payload?.kind==="busy"}function xw(e,t){return tA(e)?t("chat.deleteBusy"):e?.message||t("chat.deleteFailed")}function $w(){let e=K({queryKey:["threads"],queryFn:()=>Rx({})}),[t,a]=p.default.useState(null),[n,r]=p.default.useState(!1),s=p.default.useRef(new Map),i=p.default.useCallback(async c=>{let d=c||"__global__",f=s.current.get(d);if(f)return f;r(!0);let m=(async()=>{try{let h=await fc(c?{projectId:c}:void 0);At.invalidateQueries({queryKey:["threads"]});let b=h?.thread?.thread_id;return b&&a(b),b}finally{s.current.delete(d),r(s.current.size>0)}})();return s.current.set(d,m),m},[]),o=p.default.useCallback(async c=>{await Cx({threadId:c}),t===c&&a(null),At.invalidateQueries({queryKey:["threads"]})},[t]);return{threads:p.default.useMemo(()=>(e.data?.threads||[]).map(d=>({...d,id:d.thread_id,state:d.state||null,turn_count:d.turn_count||0,updated_at:d.updated_at||null})),[e.data]),nextCursor:e.data?.next_cursor||null,activeThreadId:t,setActiveThreadId:a,isLoading:e.isLoading,isCreating:n,createThread:i,deleteThread:o}}var ww={attach:l`<path
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
      ${ww[e]||ww.spark}
    </svg>
  `}function V(...e){let t=[];for(let a of e)if(a){if(typeof a=="string")t.push(a);else if(Array.isArray(a)){let n=V(...a);n&&t.push(n)}else if(typeof a=="object")for(let[n,r]of Object.entries(a))r&&t.push(n)}return t.join(" ")}function Sw(e){return e?.display_name||e?.email||e?.id||"IronClaw"}function aA(e){return Sw(e).trim().charAt(0).toUpperCase()||"I"}function nA(){let[e,t]=p.default.useState(!1),a=p.default.useCallback(()=>{t(n=>!n)},[]);return{open:e,toggle:a}}function Nw({theme:e,toggleTheme:t,profile:a,onSignOut:n}){let r=R(),s=nA(),i=Sw(a),o=a?.email||a?.role||r("common.gatewaySession");return l`
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
            />`:l`<span className="place-self-center">${aA(a)}</span>`}
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
  `}var _w={chat:"chat",workspace:"layers",projects:"folder",jobs:"pulse",routines:"clock",automations:"calendar",missions:"flag",extensions:"plug",logs:"list",settings:"settings",admin:"shield"},rA=Yo.filter(e=>e.id!=="chat"&&!e.hidden);function sA({route:e,label:t,onNavigate:a}){return l`
    <${Qa}
      to=${e.path}
      onClick=${a}
      className=${({isActive:n})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",n?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
    >
      <${D} name=${_w[e.id]||"bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${t}</span>
    <//>
  `}function iA({route:e,label:t,subRoutes:a,onNavigate:n}){let r=R(),s=Pe(),i=s.pathname===e.path||s.pathname.startsWith(e.path+"/"),o=`${e.path}/${a[0].id}`;return l`
    <div className="flex flex-col">
      <${Qa}
        to=${o}
        onClick=${n}
        className=${()=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",i?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
      >
        <${D}
          name=${_w[e.id]||"bolt"}
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
  `}function kw({onNewChat:e,isCreating:t,isAdmin:a=!1,onNavigate:n}){let r=R(),s=p.default.useMemo(()=>rA.filter(i=>a||i.id!=="admin"),[a]);return l`
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
        ${s.map(i=>{let o=(gc[i.id]||[]).filter(u=>a||!(i.id==="settings"&&["users","inference"].includes(u.id)));return o.length>0?l`
              <${iA}
                key=${i.id}
                route=${i}
                label=${r(i.labelKey)}
                subRoutes=${o}
                onNavigate=${n}
              />
            `:l`
            <${sA}
              key=${i.id}
              route=${i}
              label=${r(i.labelKey)}
              onNavigate=${n}
            />
          `})}
      </nav>
    </div>
  `}var Sn=Object.freeze({RUNNING:"running",NEEDS_ATTENTION:"needs_attention",FAILED:"failed"}),Zo=new Set([Sn.NEEDS_ATTENTION,Sn.FAILED]),Yp="ironclaw:v2-thread-attention",Jp=new Set,ni=new Map;function oA(){try{let e=window.localStorage.getItem(Yp);if(!e)return[];let t=JSON.parse(e);return Array.isArray(t)?t.filter(a=>Array.isArray(a)&&typeof a[0]=="string"&&Zo.has(a[1])):[]}catch{return[]}}function Rw(){let e=[];for(let[t,a]of ni)Zo.has(a)&&e.push([t,a]);try{e.length===0?window.localStorage.removeItem(Yp):window.localStorage.setItem(Yp,JSON.stringify(e))}catch{}}for(let[e,t]of oA())ni.set(e,t);function Ew(){return new Map(ni)}function Cw(){let e=Ew();for(let t of Jp)try{t(e)}catch{}}function Sc(e,t){if(!e)return;let a=ni.get(e);if(t==null){if(!ni.delete(e))return;Zo.has(a)&&Rw(),Cw();return}a!==t&&(ni.set(e,t),(Zo.has(t)||Zo.has(a))&&Rw(),Cw())}function Tw(e){Sc(e,null)}function lA(){return Ew()}function uA(e){return Jp.add(e),()=>{Jp.delete(e)}}function Aw(){let[e,t]=p.default.useState(lA);return p.default.useEffect(()=>uA(t),[]),e}function Nc(e){return e.updated_at||e.created_at||null}function Xp(e,t){let a=Nc(e)||"",n=Nc(t)||"";return a===n?(e.id||"").localeCompare(t.id||""):n.localeCompare(a)}function Dw(e){if(!e)return"";let t=new Date(e),a=new Date;return t.toDateString()===a.toDateString()?t.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"}):t.toLocaleDateString([],{month:"short",day:"numeric"})}function Mw(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):""}function cA(){let[e,t]=p.default.useState(s$);return p.default.useEffect(()=>i$(t),[]),e}var dA=Object.freeze({[Sn.NEEDS_ATTENTION]:{label:"Needs your attention",textClass:"text-[var(--v2-warning-text)]",dotClass:"bg-[var(--v2-warning-text)]",borderClass:"border-transparent"},[Sn.RUNNING]:{label:"Running",textClass:"text-[var(--v2-positive-text)]",dotClass:"bg-[var(--v2-positive-text)]",borderClass:"border-[var(--v2-positive-text)]"},[Sn.FAILED]:{label:"Failed",textClass:"text-[var(--v2-danger-text)]",dotClass:"bg-[var(--v2-danger-text)]",borderClass:"border-[var(--v2-danger-text)]"}});function mA(e){return e&&dA[e]||null}function fA({thread:e,isActive:t,isPinned:a,presentation:n,onSelect:r,onDelete:s}){let i=R(),o=Nc(e),u=Dw(o),c=Mw(o),d=p.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),window.confirm("Delete this chat?")&&Promise.resolve(s?.(e.id)).catch(h=>{window.alert(h?.message||"Unable to delete chat")})},[s,e.id]),f=p.default.useCallback(m=>{m.preventDefault(),m.stopPropagation(),r$(e.id)},[e.id]);return l`
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
  `}function Ow({label:e,items:t,activeThreadId:a,states:n,pinnedIds:r,onSelect:s,onDelete:i}){return t.length===0?null:l`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${e}
      </span>
      ${t.map(o=>l`
          <${fA}
            key=${o.id}
            thread=${o}
            isActive=${o.id===a}
            isPinned=${r.has(o.id)}
            presentation=${mA(n.get(o.id))}
            onSelect=${s}
            onDelete=${i}
          />
        `)}
    </div>
  `}function Lw({threads:e,activeThreadId:t,rebornProjectsEnabled:a=!1,onSelect:n,onDelete:r,onNavigate:s}){let[i,o]=p.default.useState(!1),[u,c]=p.default.useState(""),d=Aw(),f=cA(),m=R(),{pinned:h,recent:b,totalMatches:y}=p.default.useMemo(()=>{let w=u.trim().toLowerCase(),g=w?e.filter($=>($.title||$.id||"").toLowerCase().includes(w)):e,v=[],x=[];for(let $ of g)f.has($.id)?v.push($):x.push($);return v.sort(Xp),x.sort(Xp),{pinned:v,recent:x,totalMatches:v.length+x.length}},[e,u,f]);return l`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${()=>o(w=>!w)}
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
            onInput=${w=>c(w.currentTarget.value)}
            placeholder=${m("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${a&&l`<div className="mb-1 px-1">
          <${Qa}
            to="/projects"
            onClick=${s}
            className=${({isActive:w})=>V("flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",w?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
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

          <${Ow}
            label=${m("common.pinned")}
            items=${h}
            activeThreadId=${t}
            states=${d}
            pinnedIds=${f}
            onSelect=${n}
            onDelete=${r}
          />
          <${Ow}
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
  `}function _c(){let e=J(),t=K({queryKey:["trace-credits"],queryFn:Z$,refetchInterval:3e5,refetchIntervalInBackground:!1,refetchOnWindowFocus:!0,staleTime:6e4}),a=Q({mutationFn:W$,onSuccess:()=>e.invalidateQueries({queryKey:["trace-credits"]})});return{credits:t.data||null,query:t,authorize:a}}function pA(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Pw(){let e=R(),{credits:t}=_c();if(!t||!t.enrolled)return null;let a=pA(t.final_credit),n=t.submissions_accepted||0,r=t.submissions_submitted||0,s=t.manual_review_hold_count||0;return l`
    <div className="px-3 pb-1">
      <${$n}
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
  `}function Uw({id:e,threadsState:t,theme:a,toggleTheme:n,profile:r,isAdmin:s,rebornProjectsEnabled:i=!1,onSignOut:o,onClose:u,onNewChat:c,onSelectThread:d,onDeleteThread:f}){return l`
    <aside
      id=${e}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${$n}
          to="/chat"
          onClick=${u}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${kw}
        onNewChat=${c}
        isCreating=${t.isCreating}
        isAdmin=${s}
        onNavigate=${u}
      />

      <${Pw} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${Lw}
          threads=${t.threads}
          activeThreadId=${t.activeThreadId}
          rebornProjectsEnabled=${i}
          onSelect=${d}
          onDelete=${f}
          onNavigate=${u}
        />
      </div>

      <${Nw}
        theme=${a}
        toggleTheme=${n}
        profile=${r}
        onSignOut=${o}
      />
    </aside>
  `}var hA="radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)",vA="radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)",jw="inline-flex items-center justify-center font-semibold select-none disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]",Fw={sm:"h-9 rounded-[10px] px-3 text-xs",md:"min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"min-h-[54px] rounded-[18px] px-6 text-base",icon:"h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]","icon-sm":"h-9 w-9 rounded-[10px]"},Bw={outline:"border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] active:bg-[rgba(76,167,230,0.15)]",secondary:"border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",ghost:"border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",danger:"border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]"};function A({children:e,className:t="",variant:a="primary",size:n="md",fullWidth:r=!1,as:s="button",...i}){let o=Fw[n]??Fw.md,u=r?"w-full":"";if(a==="primary")return l`
      <${s}
        style=${{background:hA,border:"1px solid rgba(76, 167, 230, 0.72)"}}
        className=${V(jw,o,u,"relative overflow-hidden text-white group","hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",t)}
        ...${i}
      >
        <span
          aria-hidden="true"
          style=${{background:vA}}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          ${e}
        </span>
      <//>
    `;let c=Bw[a]??Bw.outline;return l`
    <${s}
      className=${V(jw,o,u,c,t)}
      ...${i}
    >
      ${e}
    <//>
  `}function zw(){let e=p.default.useMemo(()=>gA(window.location),[]),[t,a]=p.default.useState(null),[n,r]=p.default.useState(null),[s,i]=p.default.useState(!1),[o,u]=p.default.useState(""),[c,d]=p.default.useState(!1);p.default.useEffect(()=>{if(!e)return;let h=new AbortController;return fetch(`${e.base}/instances/${encodeURIComponent(e.instance)}/attestation`,{signal:h.signal}).then(b=>{if(!b.ok)throw new Error(String(b.status));return b.json()}).then(a).catch(()=>{h.signal.aborted||a(null)}),()=>h.abort()},[e]);let f=p.default.useCallback(async()=>{if(!e||n||s)return n;i(!0),u("");try{let h=await fetch(`${e.base}/attestation/report`);if(!h.ok)throw new Error(String(h.status));let b=await h.json();return r(b),b}catch(h){return u(h.message||"Could not load attestation report."),null}finally{i(!1)}},[e,n,s]),m=p.default.useCallback(async()=>{let h=n||await f();return!h||!navigator.clipboard?!1:(await navigator.clipboard.writeText(JSON.stringify({...h,instance_attestation:t},null,2)),d(!0),window.setTimeout(()=>d(!1),1800),!0)},[f,n,t]);return{available:!!t,teeInfo:t,report:n,reportError:o,reportLoading:s,copied:c,loadReport:f,copyReport:m}}function gA(e){let t=e.hostname;if(!t||t==="localhost"||yA(t))return null;let a=t.split(".");return a.length<2?null:{base:`${e.protocol}//api.${a.slice(1).join(".")}`,instance:a[0]}}function yA(e){return e.includes(":")||/^(\d{1,3}\.){3}\d{1,3}$/.test(e)}var bA=[["image_digest","tee.imageDigest"],["tls_certificate_fingerprint","tee.tlsFingerprint"],["report_data","tee.reportData"],["vm_config","tee.vmConfig"]];function qw(){let e=R(),t=zw(),[a,n]=p.default.useState(!1),r=p.default.useCallback(()=>{n(o=>{let u=!o;return u&&t.loadReport(),u})},[t]),s=p.default.useCallback(()=>{t.copyReport().catch(()=>{})},[t]);if(!t.available)return null;let i=xA({teeInfo:t.teeInfo,report:t.report,t:e});return l`
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
            <${A}
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
  `}function xA({teeInfo:e,report:t,t:a}){let n={...t,image_digest:e?.image_digest};return bA.map(([r,s])=>({label:a(s),value:$A(n[r])||a("common.unknown")}))}function $A(e){if(!e)return"";let t=typeof e=="string"?e:JSON.stringify(e);return t.length>72?`${t.slice(0,72)}...`:t}var wA="https://docs.ironclaw.com";function Iw({threadsState:e,onToggleSidebar:t,sidebarOpen:a=!0}){let n=R(),r=Pe(),s=p.default.useMemo(()=>{for(let o of Yo){let u=gc[o.id];if(!u)continue;let c=o.path+"/";if(r.pathname.startsWith(c)){let d=r.pathname.slice(c.length).split("/")[0],f=u.find(m=>m.id===d);if(f)return{parent:n(o.labelKey),current:n(f.labelKey)}}}return null},[r.pathname,n]),i=p.default.useMemo(()=>{if(s)return null;if(r.pathname.startsWith("/chat"))return e.activeThreadId&&e.threads.find(c=>c.id===e.activeThreadId)?.title||n("nav.chat");let o=Yo.find(u=>r.pathname.startsWith(u.path));return o?n(o.labelKey):""},[r.pathname,e.activeThreadId,e.threads,n,s]);return l`
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
        <${D} name="list" className="h-4 w-4" />
      </button>

      ${s?l`
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
          `:l`
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              ${i}
            </span>
          `}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <${qw} />
        <${Qa}
          to="/logs"
          className=${({isActive:o})=>V("inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",o&&"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]")}
          title=${n("nav.logs")}
        >
          ${n("nav.logs")}
        <//>
        <a
          href=${wA}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${n("nav.docs")}
        >
          ${n("nav.docs")}
        </a>
      </div>
    </header>
  `}function Kw({open:e,onClose:t,threadsState:a,onNewChat:n,onToggleTheme:r}){let s=fe(),i=R(),[o,u]=p.default.useState(""),[c,d]=p.default.useState(0),f=p.default.useRef(null),m=p.default.useMemo(()=>{let g=[{id:"new-chat",label:"New chat",icon:"plus",group:"Actions",run:()=>n?.()},{id:"go-chat",label:"Go to Chat",icon:"chat",group:"Navigate",run:()=>s("/chat")},{id:"go-extensions",label:"Go to Extensions",icon:"plug",group:"Navigate",run:()=>s("/extensions")},{id:"go-settings",label:"Go to Settings",icon:"settings",group:"Navigate",run:()=>s("/settings")},{id:"toggle-theme",label:"Toggle theme",icon:"moon",group:"Actions",run:()=>r?.()}],v=(a?.threads||[]).map(x=>({id:`thread-${x.id}`,label:x.title||`Thread ${x.id.slice(0,8)}`,icon:"chat",group:"Threads",run:()=>s(`/chat/${x.id}`)}));return[...g,...v]},[a,s,n,r]),h=p.default.useMemo(()=>{let g=o.trim().toLowerCase();return g?m.filter(v=>v.label.toLowerCase().includes(g)):m},[m,o]);p.default.useEffect(()=>{if(!e)return;u(""),d(0);let g=window.requestAnimationFrame(()=>f.current?.focus());return()=>window.cancelAnimationFrame(g)},[e]),p.default.useEffect(()=>{d(g=>Math.min(g,Math.max(0,h.length-1)))},[h.length]);let b=p.default.useCallback(g=>{g&&(t(),g.run())},[t]),y=p.default.useCallback(g=>{g.key==="ArrowDown"?(g.preventDefault(),d(v=>Math.min(v+1,h.length-1))):g.key==="ArrowUp"?(g.preventDefault(),d(v=>Math.max(v-1,0))):g.key==="Enter"?(g.preventDefault(),b(h[c])):g.key==="Escape"&&(g.preventDefault(),t())},[h,c,b,t]);if(!e)return null;let w=null;return l`
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
          ${h.length===0&&l`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${h.map((g,v)=>{let x=g.group!==w;return w=g.group,l`
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
  `}var Hw={info:"border-[var(--v2-panel-border)] text-[var(--v2-text)]",success:"border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",error:"border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]"},SA={info:"bolt",success:"check",error:"close"};function Qw(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>bw(a=>{t(n=>[...n,a]),setTimeout(()=>t(n=>n.filter(r=>r.id!==a.id)),a.duration)}),[]),e.length===0?null:l`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${e.map(a=>l`
          <div
            key=${a.id}
            role="status"
            className=${["pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",Hw[a.tone]||Hw.info].join(" ")}
          >
            <${D} name=${SA[a.tone]||"bolt"} className="h-4 w-4 shrink-0" />
            <span>${a.message}</span>
          </div>
        `)}
    </div>
  `}function Vw({token:e,profile:t,isChecking:a=!1,isAdmin:n,rebornProjectsEnabled:r=!1,onSignOut:s}){let i=R(),{theme:o,toggleTheme:u}=yc(),c=C$(e),d=$w(),f=yw({onNewChat:()=>d.setActiveThreadId(null)}),m=c.data,h=Pe(),b=fe(),y=ti({settings:{},gatewayStatus:m,enabled:n}),w=n&&uw({isLoading:y.isLoading,hasActiveProvider:y.hasActiveProvider,isError:y.isError}),g=h.pathname==="/welcome"||h.pathname.startsWith("/settings"),[v,x]=p.default.useState(!1);p.default.useEffect(()=>{let S=C=>{(C.metaKey||C.ctrlKey)&&C.key.toLowerCase()==="k"&&(C.preventDefault(),x(_=>!_))};return window.addEventListener("keydown",S),()=>window.removeEventListener("keydown",S)},[]);let $=p.default.useCallback(async S=>{let C=d.activeThreadId===S;try{await d.deleteThread(S),C&&b("/chat",{replace:!0})}catch(_){console.error("Failed to delete thread:",_),ai(xw(_,i),{tone:"error"})}},[b,d,i]);return w&&!g?l`<${ot} to="/welcome" replace />`:l`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${f.mobileOpen&&l`<button
        type="button"
        aria-label=${i("nav.close")}
        onClick=${f.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${V("fixed inset-y-0 left-0 z-50 md:relative md:z-auto",f.mobileOpen?"flex":"hidden",f.desktopOpen?"md:flex":"md:hidden")}
      >
        <${Uw}
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
          onDeleteThread=${$}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${Iw}
          threadsState=${d}
          onToggleSidebar=${f.toggle}
          sidebarOpen=${f.currentOpen}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${c.error&&l`
            <div
              className=${V("m-4 rounded-[14px] border px-4 py-3 text-sm","border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]","bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]")}
            >
              ${c.error.message||i("error.gatewayConnection")}
            </div>
          `}
          <${wp}
            context=${{gatewayStatus:m,gatewayStatusQuery:c,currentUser:t,isChecking:a,isAdmin:n,threadsState:d}}
          />
        </main>
      </div>
      <${Kw}
        open=${v}
        onClose=${()=>x(!1)}
        threadsState=${d}
        onNewChat=${f.newChat}
        onToggleTheme=${u}
      />
      <${Qw} />
    </div>
  `}var Ht=ze(He(),1),nl=e=>e.type==="checkbox",qr=e=>e instanceof Date,Dt=e=>e==null,i1=e=>typeof e=="object",Ge=e=>!Dt(e)&&!Array.isArray(e)&&i1(e)&&!qr(e),NA=e=>Ge(e)&&e.target?nl(e.target)?e.target.checked:e.target.value:e,_A=e=>e.substring(0,e.search(/\.\d+(\.|$)/))||e,kA=(e,t)=>e.has(_A(t)),RA=e=>{let t=e.constructor&&e.constructor.prototype;return Ge(t)&&t.hasOwnProperty("isPrototypeOf")},eh=typeof window<"u"&&typeof window.HTMLElement<"u"&&typeof document<"u";function ft(e){let t,a=Array.isArray(e),n=typeof FileList<"u"?e instanceof FileList:!1;if(e instanceof Date)t=new Date(e);else if(!(eh&&(e instanceof Blob||n))&&(a||Ge(e)))if(t=a?[]:Object.create(Object.getPrototypeOf(e)),!a&&!RA(e))t=e;else for(let r in e)e.hasOwnProperty(r)&&(t[r]=ft(e[r]));else return e;return t}var Tc=e=>/^\w*$/.test(e),We=e=>e===void 0,th=e=>Array.isArray(e)?e.filter(Boolean):[],ah=e=>th(e.replace(/["|']|\]/g,"").split(/\.|\[/)),Y=(e,t,a)=>{if(!t||!Ge(e))return a;let n=(Tc(t)?[t]:ah(t)).reduce((r,s)=>Dt(r)?r:r[s],e);return We(n)||n===e?We(e[t])?a:e[t]:n},Ga=e=>typeof e=="boolean",Ue=(e,t,a)=>{let n=-1,r=Tc(t)?[t]:ah(t),s=r.length,i=s-1;for(;++n<s;){let o=r[n],u=a;if(n!==i){let c=e[o];u=Ge(c)||Array.isArray(c)?c:isNaN(+r[n+1])?{}:[]}if(o==="__proto__"||o==="constructor"||o==="prototype")return;e[o]=u,e=e[o]}},Gw={BLUR:"blur",FOCUS_OUT:"focusout",CHANGE:"change"},Ea={onBlur:"onBlur",onChange:"onChange",onSubmit:"onSubmit",onTouched:"onTouched",all:"all"},Nn={max:"max",min:"min",maxLength:"maxLength",minLength:"minLength",pattern:"pattern",required:"required",validate:"validate"},CA=Ht.default.createContext(null);CA.displayName="HookFormContext";var EA=(e,t,a,n=!0)=>{let r={defaultValues:t._defaultValues};for(let s in e)Object.defineProperty(r,s,{get:()=>{let i=s;return t._proxyFormState[i]!==Ea.all&&(t._proxyFormState[i]=!n||Ea.all),a&&(a[i]=!0),e[i]}});return r},TA=typeof window<"u"?Ht.default.useLayoutEffect:Ht.default.useEffect;var Ya=e=>typeof e=="string",AA=(e,t,a,n,r)=>Ya(e)?(n&&t.watch.add(e),Y(a,e,r)):Array.isArray(e)?e.map(s=>(n&&t.watch.add(s),Y(a,s))):(n&&(t.watchAll=!0),a),Wp=e=>Dt(e)||!i1(e);function sr(e,t,a=new WeakSet){if(Wp(e)||Wp(t))return e===t;if(qr(e)&&qr(t))return e.getTime()===t.getTime();let n=Object.keys(e),r=Object.keys(t);if(n.length!==r.length)return!1;if(a.has(e)||a.has(t))return!0;a.add(e),a.add(t);for(let s of n){let i=e[s];if(!r.includes(s))return!1;if(s!=="ref"){let o=t[s];if(qr(i)&&qr(o)||Ge(i)&&Ge(o)||Array.isArray(i)&&Array.isArray(o)?!sr(i,o,a):i!==o)return!1}}return!0}var DA=(e,t,a,n,r)=>t?{...a[e],types:{...a[e]&&a[e].types?a[e].types:{},[n]:r||!0}}:{},tl=e=>Array.isArray(e)?e:[e],Yw=()=>{let e=[];return{get observers(){return e},next:r=>{for(let s of e)s.next&&s.next(r)},subscribe:r=>(e.push(r),{unsubscribe:()=>{e=e.filter(s=>s!==r)}}),unsubscribe:()=>{e=[]}}},Qt=e=>Ge(e)&&!Object.keys(e).length,nh=e=>e.type==="file",Ta=e=>typeof e=="function",Rc=e=>{if(!eh)return!1;let t=e?e.ownerDocument:0;return e instanceof(t&&t.defaultView?t.defaultView.HTMLElement:HTMLElement)},o1=e=>e.type==="select-multiple",rh=e=>e.type==="radio",MA=e=>rh(e)||nl(e),Zp=e=>Rc(e)&&e.isConnected;function OA(e,t){let a=t.slice(0,-1).length,n=0;for(;n<a;)e=We(e)?n++:e[t[n++]];return e}function LA(e){for(let t in e)if(e.hasOwnProperty(t)&&!We(e[t]))return!1;return!0}function Ze(e,t){let a=Array.isArray(t)?t:Tc(t)?[t]:ah(t),n=a.length===1?e:OA(e,a),r=a.length-1,s=a[r];return n&&delete n[s],r!==0&&(Ge(n)&&Qt(n)||Array.isArray(n)&&LA(n))&&Ze(e,a.slice(0,-1)),e}var l1=e=>{for(let t in e)if(Ta(e[t]))return!0;return!1};function Cc(e,t={}){let a=Array.isArray(e);if(Ge(e)||a)for(let n in e)Array.isArray(e[n])||Ge(e[n])&&!l1(e[n])?(t[n]=Array.isArray(e[n])?[]:{},Cc(e[n],t[n])):Dt(e[n])||(t[n]=!0);return t}function u1(e,t,a){let n=Array.isArray(e);if(Ge(e)||n)for(let r in e)Array.isArray(e[r])||Ge(e[r])&&!l1(e[r])?We(t)||Wp(a[r])?a[r]=Array.isArray(e[r])?Cc(e[r],[]):{...Cc(e[r])}:u1(e[r],Dt(t)?{}:t[r],a[r]):a[r]=!sr(e[r],t[r]);return a}var Wo=(e,t)=>u1(e,t,Cc(t)),Jw={value:!1,isValid:!1},Xw={value:!0,isValid:!0},c1=e=>{if(Array.isArray(e)){if(e.length>1){let t=e.filter(a=>a&&a.checked&&!a.disabled).map(a=>a.value);return{value:t,isValid:!!t.length}}return e[0].checked&&!e[0].disabled?e[0].attributes&&!We(e[0].attributes.value)?We(e[0].value)||e[0].value===""?Xw:{value:e[0].value,isValid:!0}:Xw:Jw}return Jw},d1=(e,{valueAsNumber:t,valueAsDate:a,setValueAs:n})=>We(e)?e:t?e===""?NaN:e&&+e:a&&Ya(e)?new Date(e):n?n(e):e,Zw={isValid:!1,value:null},m1=e=>Array.isArray(e)?e.reduce((t,a)=>a&&a.checked&&!a.disabled?{isValid:!0,value:a.value}:t,Zw):Zw;function Ww(e){let t=e.ref;return nh(t)?t.files:rh(t)?m1(e.refs).value:o1(t)?[...t.selectedOptions].map(({value:a})=>a):nl(t)?c1(e.refs).value:d1(We(t.value)?e.ref.value:t.value,e)}var PA=(e,t,a,n)=>{let r={};for(let s of e){let i=Y(t,s);i&&Ue(r,s,i._f)}return{criteriaMode:a,names:[...e],fields:r,shouldUseNativeValidation:n}},Ec=e=>e instanceof RegExp,el=e=>We(e)?e:Ec(e)?e.source:Ge(e)?Ec(e.value)?e.value.source:e.value:e,e1=e=>({isOnSubmit:!e||e===Ea.onSubmit,isOnBlur:e===Ea.onBlur,isOnChange:e===Ea.onChange,isOnAll:e===Ea.all,isOnTouch:e===Ea.onTouched}),t1="AsyncFunction",UA=e=>!!e&&!!e.validate&&!!(Ta(e.validate)&&e.validate.constructor.name===t1||Ge(e.validate)&&Object.values(e.validate).find(t=>t.constructor.name===t1)),jA=e=>e.mount&&(e.required||e.min||e.max||e.maxLength||e.minLength||e.pattern||e.validate),a1=(e,t,a)=>!a&&(t.watchAll||t.watch.has(e)||[...t.watch].some(n=>e.startsWith(n)&&/^\.\w+/.test(e.slice(n.length)))),al=(e,t,a,n)=>{for(let r of a||Object.keys(e)){let s=Y(e,r);if(s){let{_f:i,...o}=s;if(i){if(i.refs&&i.refs[0]&&t(i.refs[0],r)&&!n)return!0;if(i.ref&&t(i.ref,i.name)&&!n)return!0;if(al(o,t))break}else if(Ge(o)&&al(o,t))break}}};function n1(e,t,a){let n=Y(e,a);if(n||Tc(a))return{error:n,name:a};let r=a.split(".");for(;r.length;){let s=r.join("."),i=Y(t,s),o=Y(e,s);if(i&&!Array.isArray(i)&&a!==s)return{name:a};if(o&&o.type)return{name:s,error:o};if(o&&o.root&&o.root.type)return{name:`${s}.root`,error:o.root};r.pop()}return{name:a}}var FA=(e,t,a,n)=>{a(e);let{name:r,...s}=e;return Qt(s)||Object.keys(s).length>=Object.keys(t).length||Object.keys(s).find(i=>t[i]===(!n||Ea.all))},BA=(e,t,a)=>!e||!t||e===t||tl(e).some(n=>n&&(a?n===t:n.startsWith(t)||t.startsWith(n))),zA=(e,t,a,n,r)=>r.isOnAll?!1:!a&&r.isOnTouch?!(t||e):(a?n.isOnBlur:r.isOnBlur)?!e:(a?n.isOnChange:r.isOnChange)?e:!0,qA=(e,t)=>!th(Y(e,t)).length&&Ze(e,t),IA=(e,t,a)=>{let n=tl(Y(e,a));return Ue(n,"root",t[a]),Ue(e,a,n),e},kc=e=>Ya(e);function r1(e,t,a="validate"){if(kc(e)||Array.isArray(e)&&e.every(kc)||Ga(e)&&!e)return{type:a,message:kc(e)?e:"",ref:t}}var ri=e=>Ge(e)&&!Ec(e)?e:{value:e,message:""},s1=async(e,t,a,n,r,s)=>{let{ref:i,refs:o,required:u,maxLength:c,minLength:d,min:f,max:m,pattern:h,validate:b,name:y,valueAsNumber:w,mount:g}=e._f,v=Y(a,y);if(!g||t.has(y))return{};let x=o?o[0]:i,$=k=>{r&&x.reportValidity&&(x.setCustomValidity(Ga(k)?"":k||""),x.reportValidity())},S={},C=rh(i),_=nl(i),T=C||_,M=(w||nh(i))&&We(i.value)&&We(v)||Rc(i)&&i.value===""||v===""||Array.isArray(v)&&!v.length,O=DA.bind(null,y,n,S),U=(k,z,Z,re=Nn.maxLength,me=Nn.minLength)=>{let pe=k?z:Z;S[y]={type:k?re:me,message:pe,ref:i,...O(k?re:me,pe)}};if(s?!Array.isArray(v)||!v.length:u&&(!T&&(M||Dt(v))||Ga(v)&&!v||_&&!c1(o).isValid||C&&!m1(o).isValid)){let{value:k,message:z}=kc(u)?{value:!!u,message:u}:ri(u);if(k&&(S[y]={type:Nn.required,message:z,ref:x,...O(Nn.required,z)},!n))return $(z),S}if(!M&&(!Dt(f)||!Dt(m))){let k,z,Z=ri(m),re=ri(f);if(!Dt(v)&&!isNaN(v)){let me=i.valueAsNumber||v&&+v;Dt(Z.value)||(k=me>Z.value),Dt(re.value)||(z=me<re.value)}else{let me=i.valueAsDate||new Date(v),pe=bt=>new Date(new Date().toDateString()+" "+bt),Ee=i.type=="time",je=i.type=="week";Ya(Z.value)&&v&&(k=Ee?pe(v)>pe(Z.value):je?v>Z.value:me>new Date(Z.value)),Ya(re.value)&&v&&(z=Ee?pe(v)<pe(re.value):je?v<re.value:me<new Date(re.value))}if((k||z)&&(U(!!k,Z.message,re.message,Nn.max,Nn.min),!n))return $(S[y].message),S}if((c||d)&&!M&&(Ya(v)||s&&Array.isArray(v))){let k=ri(c),z=ri(d),Z=!Dt(k.value)&&v.length>+k.value,re=!Dt(z.value)&&v.length<+z.value;if((Z||re)&&(U(Z,k.message,z.message),!n))return $(S[y].message),S}if(h&&!M&&Ya(v)){let{value:k,message:z}=ri(h);if(Ec(k)&&!v.match(k)&&(S[y]={type:Nn.pattern,message:z,ref:i,...O(Nn.pattern,z)},!n))return $(z),S}if(b){if(Ta(b)){let k=await b(v,a),z=r1(k,x);if(z&&(S[y]={...z,...O(Nn.validate,z.message)},!n))return $(z.message),S}else if(Ge(b)){let k={};for(let z in b){if(!Qt(k)&&!n)break;let Z=r1(await b[z](v,a),x,z);Z&&(k={...Z,...O(z,Z.message)},$(Z.message),n&&(S[y]=k))}if(!Qt(k)&&(S[y]={ref:x,...k},!n))return S}}return $(!0),S},KA={mode:Ea.onSubmit,reValidateMode:Ea.onChange,shouldFocusError:!0};function HA(e={}){let t={...KA,...e},a={submitCount:0,isDirty:!1,isReady:!1,isLoading:Ta(t.defaultValues),isValidating:!1,isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,touchedFields:{},dirtyFields:{},validatingFields:{},errors:t.errors||{},disabled:t.disabled||!1},n={},r=Ge(t.defaultValues)||Ge(t.values)?ft(t.defaultValues||t.values)||{}:{},s=t.shouldUnregister?{}:ft(r),i={action:!1,mount:!1,watch:!1},o={mount:new Set,disabled:new Set,unMount:new Set,array:new Set,watch:new Set},u,c=0,d={isDirty:!1,dirtyFields:!1,validatingFields:!1,touchedFields:!1,isValidating:!1,isValid:!1,errors:!1},f={...d},m={array:Yw(),state:Yw()},h=t.criteriaMode===Ea.all,b=N=>E=>{clearTimeout(c),c=setTimeout(N,E)},y=async N=>{if(!t.disabled&&(d.isValid||f.isValid||N)){let E=t.resolver?Qt((await _()).errors):await M(n,!0);E!==a.isValid&&m.state.next({isValid:E})}},w=(N,E)=>{!t.disabled&&(d.isValidating||d.validatingFields||f.isValidating||f.validatingFields)&&((N||Array.from(o.mount)).forEach(L=>{L&&(E?Ue(a.validatingFields,L,E):Ze(a.validatingFields,L))}),m.state.next({validatingFields:a.validatingFields,isValidating:!Qt(a.validatingFields)}))},g=(N,E=[],L,P,F=!0,B=!0)=>{if(P&&L&&!t.disabled){if(i.action=!0,B&&Array.isArray(Y(n,N))){let G=L(Y(n,N),P.argA,P.argB);F&&Ue(n,N,G)}if(B&&Array.isArray(Y(a.errors,N))){let G=L(Y(a.errors,N),P.argA,P.argB);F&&Ue(a.errors,N,G),qA(a.errors,N)}if((d.touchedFields||f.touchedFields)&&B&&Array.isArray(Y(a.touchedFields,N))){let G=L(Y(a.touchedFields,N),P.argA,P.argB);F&&Ue(a.touchedFields,N,G)}(d.dirtyFields||f.dirtyFields)&&(a.dirtyFields=Wo(r,s)),m.state.next({name:N,isDirty:U(N,E),dirtyFields:a.dirtyFields,errors:a.errors,isValid:a.isValid})}else Ue(s,N,E)},v=(N,E)=>{Ue(a.errors,N,E),m.state.next({errors:a.errors})},x=N=>{a.errors=N,m.state.next({errors:a.errors,isValid:!1})},$=(N,E,L,P)=>{let F=Y(n,N);if(F){let B=Y(s,N,We(L)?Y(r,N):L);We(B)||P&&P.defaultChecked||E?Ue(s,N,E?B:Ww(F._f)):Z(N,B),i.mount&&y()}},S=(N,E,L,P,F)=>{let B=!1,G=!1,ie={name:N};if(!t.disabled){if(!L||P){(d.isDirty||f.isDirty)&&(G=a.isDirty,a.isDirty=ie.isDirty=U(),B=G!==ie.isDirty);let he=sr(Y(r,N),E);G=!!Y(a.dirtyFields,N),he?Ze(a.dirtyFields,N):Ue(a.dirtyFields,N,!0),ie.dirtyFields=a.dirtyFields,B=B||(d.dirtyFields||f.dirtyFields)&&G!==!he}if(L){let he=Y(a.touchedFields,N);he||(Ue(a.touchedFields,N,L),ie.touchedFields=a.touchedFields,B=B||(d.touchedFields||f.touchedFields)&&he!==L)}B&&F&&m.state.next(ie)}return B?ie:{}},C=(N,E,L,P)=>{let F=Y(a.errors,N),B=(d.isValid||f.isValid)&&Ga(E)&&a.isValid!==E;if(t.delayError&&L?(u=b(()=>v(N,L)),u(t.delayError)):(clearTimeout(c),u=null,L?Ue(a.errors,N,L):Ze(a.errors,N)),(L?!sr(F,L):F)||!Qt(P)||B){let G={...P,...B&&Ga(E)?{isValid:E}:{},errors:a.errors,name:N};a={...a,...G},m.state.next(G)}},_=async N=>{w(N,!0);let E=await t.resolver(s,t.context,PA(N||o.mount,n,t.criteriaMode,t.shouldUseNativeValidation));return w(N),E},T=async N=>{let{errors:E}=await _(N);if(N)for(let L of N){let P=Y(E,L);P?Ue(a.errors,L,P):Ze(a.errors,L)}else a.errors=E;return E},M=async(N,E,L={valid:!0})=>{for(let P in N){let F=N[P];if(F){let{_f:B,...G}=F;if(B){let ie=o.array.has(B.name),he=F._f&&UA(F._f);he&&d.validatingFields&&w([P],!0);let xt=await s1(F,o.disabled,s,h,t.shouldUseNativeValidation&&!E,ie);if(he&&d.validatingFields&&w([P]),xt[B.name]&&(L.valid=!1,E))break;!E&&(Y(xt,B.name)?ie?IA(a.errors,xt,B.name):Ue(a.errors,B.name,xt[B.name]):Ze(a.errors,B.name))}!Qt(G)&&await M(G,E,L)}}return L.valid},O=()=>{for(let N of o.unMount){let E=Y(n,N);E&&(E._f.refs?E._f.refs.every(L=>!Zp(L)):!Zp(E._f.ref))&&Ot(N)}o.unMount=new Set},U=(N,E)=>!t.disabled&&(N&&E&&Ue(s,N,E),!sr(bt(),r)),k=(N,E,L)=>AA(N,o,{...i.mount?s:We(E)?r:Ya(N)?{[N]:E}:E},L,E),z=N=>th(Y(i.mount?s:r,N,t.shouldUnregister?Y(r,N,[]):[])),Z=(N,E,L={})=>{let P=Y(n,N),F=E;if(P){let B=P._f;B&&(!B.disabled&&Ue(s,N,d1(E,B)),F=Rc(B.ref)&&Dt(E)?"":E,o1(B.ref)?[...B.ref.options].forEach(G=>G.selected=F.includes(G.value)):B.refs?nl(B.ref)?B.refs.forEach(G=>{(!G.defaultChecked||!G.disabled)&&(Array.isArray(F)?G.checked=!!F.find(ie=>ie===G.value):G.checked=F===G.value||!!F)}):B.refs.forEach(G=>G.checked=G.value===F):nh(B.ref)?B.ref.value="":(B.ref.value=F,B.ref.type||m.state.next({name:N,values:ft(s)})))}(L.shouldDirty||L.shouldTouch)&&S(N,F,L.shouldTouch,L.shouldDirty,!0),L.shouldValidate&&je(N)},re=(N,E,L)=>{for(let P in E){if(!E.hasOwnProperty(P))return;let F=E[P],B=N+"."+P,G=Y(n,B);(o.array.has(N)||Ge(F)||G&&!G._f)&&!qr(F)?re(B,F,L):Z(B,F,L)}},me=(N,E,L={})=>{let P=Y(n,N),F=o.array.has(N),B=ft(E);Ue(s,N,B),F?(m.array.next({name:N,values:ft(s)}),(d.isDirty||d.dirtyFields||f.isDirty||f.dirtyFields)&&L.shouldDirty&&m.state.next({name:N,dirtyFields:Wo(r,s),isDirty:U(N,B)})):P&&!P._f&&!Dt(B)?re(N,B,L):Z(N,B,L),a1(N,o)&&m.state.next({...a,name:N}),m.state.next({name:i.mount?N:void 0,values:ft(s)})},pe=async N=>{i.mount=!0;let E=N.target,L=E.name,P=!0,F=Y(n,L),B=he=>{P=Number.isNaN(he)||qr(he)&&isNaN(he.getTime())||sr(he,Y(s,L,he))},G=e1(t.mode),ie=e1(t.reValidateMode);if(F){let he,xt,oe=E.type?Ww(F._f):NA(N),Lt=N.type===Gw.BLUR||N.type===Gw.FOCUS_OUT,da=!jA(F._f)&&!t.resolver&&!Y(a.errors,L)&&!F._f.deps||zA(Lt,Y(a.touchedFields,L),a.isSubmitted,ie,G),$t=a1(L,o,Lt);Ue(s,L,oe),Lt?(!E||!E.readOnly)&&(F._f.onBlur&&F._f.onBlur(N),u&&u(0)):F._f.onChange&&F._f.onChange(N);let wt=S(L,oe,Lt),fr=!Qt(wt)||$t;if(!Lt&&m.state.next({name:L,type:N.type,values:ft(s)}),da)return(d.isValid||f.isValid)&&(t.mode==="onBlur"?Lt&&y():Lt||y()),fr&&m.state.next({name:L,...$t?{}:wt});if(!Lt&&$t&&m.state.next({...a}),t.resolver){let{errors:dl}=await _([L]);if(B(oe),P){let ml=n1(a.errors,n,L),fl=n1(dl,n,ml.name||L);he=fl.error,L=fl.name,xt=Qt(dl)}}else w([L],!0),he=(await s1(F,o.disabled,s,h,t.shouldUseNativeValidation))[L],w([L]),B(oe),P&&(he?xt=!1:(d.isValid||f.isValid)&&(xt=await M(n,!0)));P&&(F._f.deps&&je(F._f.deps),C(L,xt,he,wt))}},Ee=(N,E)=>{if(Y(a.errors,E)&&N.focus)return N.focus(),1},je=async(N,E={})=>{let L,P,F=tl(N);if(t.resolver){let B=await T(We(N)?N:F);L=Qt(B),P=N?!F.some(G=>Y(B,G)):L}else N?(P=(await Promise.all(F.map(async B=>{let G=Y(n,B);return await M(G&&G._f?{[B]:G}:G)}))).every(Boolean),!(!P&&!a.isValid)&&y()):P=L=await M(n);return m.state.next({...!Ya(N)||(d.isValid||f.isValid)&&L!==a.isValid?{}:{name:N},...t.resolver||!N?{isValid:L}:{},errors:a.errors}),E.shouldFocus&&!P&&al(n,Ee,N?F:o.mount),P},bt=N=>{let E={...i.mount?s:r};return We(N)?E:Ya(N)?Y(E,N):N.map(L=>Y(E,L))},pt=(N,E)=>({invalid:!!Y((E||a).errors,N),isDirty:!!Y((E||a).dirtyFields,N),error:Y((E||a).errors,N),isValidating:!!Y(a.validatingFields,N),isTouched:!!Y((E||a).touchedFields,N)}),at=N=>{N&&tl(N).forEach(E=>Ze(a.errors,E)),m.state.next({errors:N?a.errors:{}})},Ye=(N,E,L)=>{let P=(Y(n,N,{_f:{}})._f||{}).ref,F=Y(a.errors,N)||{},{ref:B,message:G,type:ie,...he}=F;Ue(a.errors,N,{...he,...E,ref:P}),m.state.next({name:N,errors:a.errors,isValid:!1}),L&&L.shouldFocus&&P&&P.focus&&P.focus()},Rn=(N,E)=>Ta(N)?m.state.subscribe({next:L=>"values"in L&&N(k(void 0,E),L)}):k(N,E,!0),Da=N=>m.state.subscribe({next:E=>{BA(N.name,E.name,N.exact)&&FA(E,N.formState||d,Te,N.reRenderRoot)&&N.callback({values:{...s},...a,...E,defaultValues:r})}}).unsubscribe,Cn=N=>(i.mount=!0,f={...f,...N.formState},Da({...N,formState:f})),Ot=(N,E={})=>{for(let L of N?tl(N):o.mount)o.mount.delete(L),o.array.delete(L),E.keepValue||(Ze(n,L),Ze(s,L)),!E.keepError&&Ze(a.errors,L),!E.keepDirty&&Ze(a.dirtyFields,L),!E.keepTouched&&Ze(a.touchedFields,L),!E.keepIsValidating&&Ze(a.validatingFields,L),!t.shouldUnregister&&!E.keepDefaultValue&&Ze(r,L);m.state.next({values:ft(s)}),m.state.next({...a,...E.keepDirty?{isDirty:U()}:{}}),!E.keepIsValid&&y()},Za=({disabled:N,name:E})=>{(Ga(N)&&i.mount||N||o.disabled.has(E))&&(N?o.disabled.add(E):o.disabled.delete(E))},ua=(N,E={})=>{let L=Y(n,N),P=Ga(E.disabled)||Ga(t.disabled);return Ue(n,N,{...L||{},_f:{...L&&L._f?L._f:{ref:{name:N}},name:N,mount:!0,...E}}),o.mount.add(N),L?Za({disabled:Ga(E.disabled)?E.disabled:t.disabled,name:N}):$(N,!0,E.value),{...P?{disabled:E.disabled||t.disabled}:{},...t.progressive?{required:!!E.required,min:el(E.min),max:el(E.max),minLength:el(E.minLength),maxLength:el(E.maxLength),pattern:el(E.pattern)}:{},name:N,onChange:pe,onBlur:pe,ref:F=>{if(F){ua(N,E),L=Y(n,N);let B=We(F.value)&&F.querySelectorAll&&F.querySelectorAll("input,select,textarea")[0]||F,G=MA(B),ie=L._f.refs||[];if(G?ie.find(he=>he===B):B===L._f.ref)return;Ue(n,N,{_f:{...L._f,...G?{refs:[...ie.filter(Zp),B,...Array.isArray(Y(r,N))?[{}]:[]],ref:{type:B.type,name:N}}:{ref:B}}}),$(N,!1,void 0,B)}else L=Y(n,N,{}),L._f&&(L._f.mount=!1),(t.shouldUnregister||E.shouldUnregister)&&!(kA(o.array,N)&&i.action)&&o.unMount.add(N)}}},Wa=()=>t.shouldFocusError&&al(n,Ee,o.mount),en=N=>{Ga(N)&&(m.state.next({disabled:N}),al(n,(E,L)=>{let P=Y(n,L);P&&(E.disabled=P._f.disabled||N,Array.isArray(P._f.refs)&&P._f.refs.forEach(F=>{F.disabled=P._f.disabled||N}))},0,!1))},ht=(N,E)=>async L=>{let P;L&&(L.preventDefault&&L.preventDefault(),L.persist&&L.persist());let F=ft(s);if(m.state.next({isSubmitting:!0}),t.resolver){let{errors:B,values:G}=await _();a.errors=B,F=ft(G)}else await M(n);if(o.disabled.size)for(let B of o.disabled)Ze(F,B);if(Ze(a.errors,"root"),Qt(a.errors)){m.state.next({errors:{}});try{await N(F,L)}catch(B){P=B}}else E&&await E({...a.errors},L),Wa(),setTimeout(Wa);if(m.state.next({isSubmitted:!0,isSubmitting:!1,isSubmitSuccessful:Qt(a.errors)&&!P,submitCount:a.submitCount+1,errors:a.errors}),P)throw P},ca=(N,E={})=>{Y(n,N)&&(We(E.defaultValue)?me(N,ft(Y(r,N))):(me(N,E.defaultValue),Ue(r,N,ft(E.defaultValue))),E.keepTouched||Ze(a.touchedFields,N),E.keepDirty||(Ze(a.dirtyFields,N),a.isDirty=E.defaultValue?U(N,ft(Y(r,N))):U()),E.keepError||(Ze(a.errors,N),d.isValid&&y()),m.state.next({...a}))},Vt=(N,E={})=>{let L=N?ft(N):r,P=ft(L),F=Qt(N),B=F?r:P;if(E.keepDefaultValues||(r=L),!E.keepValues){if(E.keepDirtyValues){let G=new Set([...o.mount,...Object.keys(Wo(r,s))]);for(let ie of Array.from(G))Y(a.dirtyFields,ie)?Ue(B,ie,Y(s,ie)):me(ie,Y(B,ie))}else{if(eh&&We(N))for(let G of o.mount){let ie=Y(n,G);if(ie&&ie._f){let he=Array.isArray(ie._f.refs)?ie._f.refs[0]:ie._f.ref;if(Rc(he)){let xt=he.closest("form");if(xt){xt.reset();break}}}}if(E.keepFieldsRef)for(let G of o.mount)me(G,Y(B,G));else n={}}s=t.shouldUnregister?E.keepDefaultValues?ft(r):{}:ft(B),m.array.next({values:{...B}}),m.state.next({values:{...B}})}o={mount:E.keepDirtyValues?o.mount:new Set,unMount:new Set,array:new Set,disabled:new Set,watch:new Set,watchAll:!1,focus:""},i.mount=!d.isValid||!!E.keepIsValid||!!E.keepDirtyValues,i.watch=!!t.shouldUnregister,m.state.next({submitCount:E.keepSubmitCount?a.submitCount:0,isDirty:F?!1:E.keepDirty?a.isDirty:!!(E.keepDefaultValues&&!sr(N,r)),isSubmitted:E.keepIsSubmitted?a.isSubmitted:!1,dirtyFields:F?{}:E.keepDirtyValues?E.keepDefaultValues&&s?Wo(r,s):a.dirtyFields:E.keepDefaultValues&&N?Wo(r,N):E.keepDirty?a.dirtyFields:{},touchedFields:E.keepTouched?a.touchedFields:{},errors:E.keepErrors?a.errors:{},isSubmitSuccessful:E.keepIsSubmitSuccessful?a.isSubmitSuccessful:!1,isSubmitting:!1,defaultValues:r})},ae=(N,E)=>Vt(Ta(N)?N(s):N,E),ee=(N,E={})=>{let L=Y(n,N),P=L&&L._f;if(P){let F=P.refs?P.refs[0]:P.ref;F.focus&&(F.focus(),E.shouldSelect&&Ta(F.select)&&F.select())}},Te=N=>{a={...a,...N}},lt={control:{register:ua,unregister:Ot,getFieldState:pt,handleSubmit:ht,setError:Ye,_subscribe:Da,_runSchema:_,_focusError:Wa,_getWatch:k,_getDirty:U,_setValid:y,_setFieldArray:g,_setDisabledField:Za,_setErrors:x,_getFieldArray:z,_reset:Vt,_resetDefaultValues:()=>Ta(t.defaultValues)&&t.defaultValues().then(N=>{ae(N,t.resetOptions),m.state.next({isLoading:!1})}),_removeUnmounted:O,_disableForm:en,_subjects:m,_proxyFormState:d,get _fields(){return n},get _formValues(){return s},get _state(){return i},set _state(N){i=N},get _defaultValues(){return r},get _names(){return o},set _names(N){o=N},get _formState(){return a},get _options(){return t},set _options(N){t={...t,...N}}},subscribe:Cn,trigger:je,register:ua,handleSubmit:ht,watch:Rn,setValue:me,getValues:bt,reset:ae,resetField:ca,clearErrors:at,unregister:Ot,setError:Ye,setFocus:ee,getFieldState:pt};return{...lt,formControl:lt}}function f1(e={}){let t=Ht.default.useRef(void 0),a=Ht.default.useRef(void 0),[n,r]=Ht.default.useState({isDirty:!1,isValidating:!1,isLoading:Ta(e.defaultValues),isSubmitted:!1,isSubmitting:!1,isSubmitSuccessful:!1,isValid:!1,submitCount:0,dirtyFields:{},touchedFields:{},validatingFields:{},errors:e.errors||{},disabled:e.disabled||!1,isReady:!1,defaultValues:Ta(e.defaultValues)?void 0:e.defaultValues});if(!t.current)if(e.formControl)t.current={...e.formControl,formState:n},e.defaultValues&&!Ta(e.defaultValues)&&e.formControl.reset(e.defaultValues,e.resetOptions);else{let{formControl:i,...o}=HA(e);t.current={...o,formState:n}}let s=t.current.control;return s._options=e,TA(()=>{let i=s._subscribe({formState:s._proxyFormState,callback:()=>r({...s._formState}),reRenderRoot:!0});return r(o=>({...o,isReady:!0})),s._formState.isReady=!0,i},[s]),Ht.default.useEffect(()=>s._disableForm(e.disabled),[s,e.disabled]),Ht.default.useEffect(()=>{e.mode&&(s._options.mode=e.mode),e.reValidateMode&&(s._options.reValidateMode=e.reValidateMode)},[s,e.mode,e.reValidateMode]),Ht.default.useEffect(()=>{e.errors&&(s._setErrors(e.errors),s._focusError())},[s,e.errors]),Ht.default.useEffect(()=>{e.shouldUnregister&&s._subjects.state.next({values:s._getWatch()})},[s,e.shouldUnregister]),Ht.default.useEffect(()=>{if(s._proxyFormState.isDirty){let i=s._getDirty();i!==n.isDirty&&s._subjects.state.next({isDirty:i})}},[s,n.isDirty]),Ht.default.useEffect(()=>{e.values&&!sr(e.values,a.current)?(s._reset(e.values,{keepFieldsRef:!0,...s._options.resetOptions}),a.current=e.values,r(i=>({...i}))):s._resetDefaultValues()},[s,e.values]),Ht.default.useEffect(()=>{s._state.mount||(s._setValid(),s._state.mount=!0),s._state.watch&&(s._state.watch=!1,s._subjects.state.next({...s._formState})),s._removeUnmounted()}),t.current.formState=EA(n,s),t.current}var p1={default:"bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",bordered:"bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",subtle:"bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",inset:"bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]"},h1={sm:"rounded-[14px]",md:"rounded-[1.25rem] md:rounded-[1.5rem]",lg:"rounded-[1.5rem]"},QA={none:"",sm:"p-4",md:"p-5",lg:"p-5 md:p-7"};function te({children:e,className:t="",variant:a="default",radius:n="md",padding:r="none",as:s="div",...i}){return l`
    <${s}
      className=${V(p1[a]??p1.default,h1[n]??h1.md,QA[r]??"",t)}
      ...${i}
    >
      ${e}
    <//>
  `}var sh="w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)] border-[var(--v2-panel-border)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",Ac={sm:"h-9 rounded-[10px] px-3 text-[12px]",md:"h-[44px] rounded-[14px] px-3.5 text-[13px] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm",lg:"h-[54px] rounded-[18px] px-4 text-base"};function Mt({className:e="",size:t="md",error:a=!1,...n}){return l`
    <input
      className=${V(sh,Ac[t]??Ac.md,a&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function Dc({className:e="",error:t=!1,rows:a=4,...n}){return l`
    <textarea
      rows=${a}
      className=${V(sh,"rounded-[14px] px-3.5 py-3 text-[13px] md:rounded-[16px] md:px-4 md:text-sm","resize-y min-h-[80px]",t&&"border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",e)}
      ...${n}
    />
  `}function ih({children:e,className:t="",size:a="md",error:n=!1,...r}){return l`
    <div className="relative w-full">
      <select
        className=${V(sh,Ac[a]??Ac.md,"appearance-none pr-9 cursor-pointer",n&&"border-[var(--v2-danger-text)]",t)}
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
  `}function VA({children:e,className:t="",required:a=!1,...n}){return l`
    <label
      className=${V("block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",t)}
      ...${n}
    >
      ${e}
      ${a&&l`<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>`}
    </label>
  `}function _n({label:e,children:t,error:a="",hint:n="",required:r=!1,className:s="",htmlFor:i=""}){return l`
    <div className=${V("flex flex-col gap-2",s)}>
      ${e&&l`<${VA} htmlFor=${i} required=${r}>${e}<//>`}
      ${t}
      ${a&&l`<p className="text-xs text-[var(--v2-danger-text)]" role="alert">${a}</p>`}
      ${!a&&n&&l`<p className="text-xs text-[var(--v2-text-faint)]">${n}</p>`}
    </div>
  `}var GA={google:"Google",github:"GitHub",apple:"Apple"};function YA(e,t){return`/auth/login/${encodeURIComponent(e)}?redirect_after=${encodeURIComponent(t)}`}function v1({providers:e,redirectAfter:t}){let a=R();return e.length?l`
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
              href=${YA(n,t)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <${D} name="shield" className="h-4 w-4" />
              ${a("login.oauthProvider",{provider:GA[n]||n})}
            <//>
          `)}
      </div>
    </div>
  `:null}var JA=["google","github","apple"];function g1(){let[e,t]=p.default.useState([]);return p.default.useEffect(()=>{let a=!1;return Yx().then(n=>{if(a)return;let r=Array.isArray(n?.providers)?n.providers:[];t(JA.filter(s=>r.includes(s)))}).catch(()=>{a||t([])}),()=>{a=!0}},[]),e}function y1({initialToken:e,error:t,oauthRedirectAfter:a="/v2",onSubmit:n}){let r=R(),{theme:s,toggleTheme:i}=yc(),o=g1(),{formState:{errors:u,isSubmitting:c},handleSubmit:d,register:f}=f1({defaultValues:{token:e||""}});return l`
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
        <${D} name=${s==="dark"?"sun":"moon"} className="h-4 w-4" />
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
          <${_n}
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

          <${A}
            type="submit"
            variant="primary"
            fullWidth
            disabled=${c}
          >
            ${r("login.connect")}
          <//>
        </form>

        <${v1}
          providers=${o}
          redirectAfter=${a}
        />
      <//>
    </main>
  `}var b1={success:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",positive:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",signal:"border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",warning:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",copper:"border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",danger:"border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",info:"border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",accent:"border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",muted:"border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"},x1={sm:"h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",md:"h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]"};function q({tone:e="muted",label:t,dot:a=!0,size:n="md",className:r=""}){let s=e==="success"||e==="positive"||e==="signal";return l`
    <span
      className=${V("inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",x1[n]??x1.md,b1[e]??b1.muted,r)}
    >
      ${a&&l`<span
          className=${V("h-1.5 w-1.5 shrink-0 rounded-full bg-current",s&&"animate-[v2-breathe_2s_ease-in-out_infinite]")}
        />`}
      ${t}
    </span>
  `}var XA=/(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/,w1=/(bash|shell|exec|run|command|terminal|spawn|process)/,$1=/(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;function S1(e,t,a){let n=String(e||"").toLowerCase(),r=[t,a].filter(Boolean).join(" ").toLowerCase();return XA.test(n)?{tone:"danger",key:"tool.riskWrite"}:w1.test(n)?{tone:"warning",key:"tool.riskExec"}:$1.test(n)?{tone:"info",key:"tool.riskNetwork"}:ZA(r)?{tone:"warning",key:"tool.riskExec"}:$1.test(r)?{tone:"info",key:"tool.riskNetwork"}:{tone:"muted",key:"tool.riskRead"}}function ZA(e){if(!e)return!1;let t=e.replace(/\bcontinue the run\b/g,"").replace(/\bresolve this (approval )?gate\b/g,"");return w1.test(t)}var Mc=480;function WA(e,t){return t&&t.length>0?t.some(a=>typeof a?.value=="string"&&a.value.length>Mc):typeof e=="string"&&e.length>Mc}function N1(e,t){return typeof e!="string"||t||e.length<=Mc?e:`${e.slice(0,Mc).trimEnd()}
...`}function _1({gate:e,onApprove:t,onDeny:a,onAlways:n}){let r=R(),{toolName:s,description:i,parameters:o,allowAlways:u,approvalDetails:c=[]}=e,[d,f]=p.default.useState(!1),[m,h]=p.default.useState(!1);p.default.useEffect(()=>{h(!1)},[e]);let b=p.default.useMemo(()=>S1(s,i,o),[s,i,o]),y=s||r("approval.thisTool"),w=WA(o,c),g=m?"max-h-72":"max-h-36",v=p.default.useCallback(()=>{d&&u?n?.():t?.()},[d,u,n,t]);return l`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${D} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${r("approval.title")}</span>
        <${q}
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
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${N1(x.value,m)}</dd>
                  </div>
                `)}
            </dl>
          `:o&&l`<pre className=${`mb-2 ${g} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${N1(o,m)}</pre>`}

      ${w&&l`
        <${A}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${()=>h(x=>!x)}
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
        <${A} variant="primary" onClick=${v}>
          ${r(d&&u?"approval.approveAndAlways":"approval.approve")}
        <//>
        <${A} variant="secondary" onClick=${()=>a?.()}>
          ${r("approval.deny")}
        <//>
      </div>
    </div>
  `}function si({icon:e="lock",headline:t,provider:a,accountLabel:n,body:r,expiresAt:s,pillHint:i,defaultExpanded:o=!0,children:u}){let c=R(),[d,f]=p.default.useState(o),m=p.default.useId(),h=n||a||"";return l`
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
          ${h&&l`<span className="block truncate text-xs text-iron-300">${h}</span>`}
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
  `}function k1({gate:e,onCancel:t}){let a=R();return l`
    <${si}
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
  `}function R1({gate:e,onCancel:t}){let a=R(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),o=p.default.useMemo(()=>{if(!e.authorizationUrl)return!1;try{return new URL(e.authorizationUrl).protocol==="https:"}catch{return!1}},[e.authorizationUrl]);p.default.useEffect(()=>{i("")},[e.authorizationUrl,e.gateRef,e.runId]);let u=e.provider?e.provider.charAt(0).toUpperCase()+e.provider.slice(1):a("authGate.oauthProviderFallback"),c=p.default.useCallback(()=>{if(!o){i(a("authGate.serviceUnavailable"));return}i(""),window.open(e.authorizationUrl,"_blank","noopener,noreferrer"),r(!0)},[e.authorizationUrl,o]),d=n?a("authGate.reopenAuthorization",{provider:u}):a("authGate.openAuthorization",{provider:u});return l`
    <${si}
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
          onClick=${f=>{f.preventDefault(),c()}}
        >
          <${D} name="link" className="h-4 w-4" />
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
  `}function C1({gate:e,onSubmit:t,onCancel:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[u,c]=p.default.useState(!1),d=p.default.useCallback(async f=>{f.preventDefault();let m=r.trim();if(!m){o(n("authGate.tokenRequired"));return}o(""),c(!0);try{await t(m),s("")}catch(h){o(h?.safeAuthGateCode==="credential_stored_gate_resolution_failed"?n("authGate.resolveFailedAfterTokenSaved"):n("authGate.submitFailed"))}finally{c(!1)}},[t,n,r]);return l`
    <${si}
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
  `}var e4="/api/webchat/v2/extensions/pairing/redeem";function E1(e){return H(e4,{method:"POST",body:JSON.stringify({channel:"slack",code:e})}).then(t=>({success:!0,provider:t.provider,provider_user_id:t.provider_user_id,message:"Slack account connected."}))}function Oc({action:e}){let t=R(),a=J(),n=Q({mutationFn:({code:u})=>E1(u),onSuccess:()=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["pairing","slack"]})}}),[r,s]=p.default.useState(""),i=t4(e,t),o=()=>{let u=r.trim();u&&(n.mutate({code:u}),s(""))};return l`
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
        ${a4(n.error,i.errorMessage)}
      </p>`}
    </div>
  `}function t4(e,t){return{title:e?.title||t("pairing.slackTitle"),instructions:e?.instructions||t("pairing.slackInstructions"),codePlaceholder:e?.input_placeholder||e?.code_placeholder||t("pairing.slackPlaceholder"),submitLabel:e?.submit_label||t("pairing.connect"),successMessage:e?.success_message||t("pairing.slackSuccess"),errorMessage:e?.error_message||t("pairing.slackError")}}function a4(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function n4(e,t){return e?.channel==="slack"&&e.strategy===t}function T1({connectAction:e,onDismiss:t}){if(!e)return null;let a=e.channel;return l`
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

      ${n4(e,"inbound_proof_code")?l`<${Oc} action=${e.action} />`:l`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${e.action?.instructions||"This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `}function r4(e){let t=e?.attachments;return t?{accept:Array.isArray(t.accept)?t.accept.filter(a=>typeof a=="string"):Fr.accept,maxCount:Number.isFinite(t.max_count)?t.max_count:Fr.maxCount,maxFileBytes:Number.isFinite(t.max_file_bytes)?t.max_file_bytes:Fr.maxFileBytes,maxTotalBytes:Number.isFinite(t.max_total_bytes)?t.max_total_bytes:Fr.maxTotalBytes}:Fr}function A1(){let e=Sa(),t=K({enabled:!!e,queryKey:["session"],queryFn:mc,staleTime:5*6e4});return r4(t.data)}function Lc({onSend:e,onCancel:t,disabled:a,sendDisabled:n=a,canCancel:r=!1,initialText:s="",resetKey:i="",draftKey:o=Zs,autoFocusKey:u="",variant:c="dock",context:d={},statusText:f=""}){let m=R(),h=c==="hero",b=A1(),[y,w]=p.default.useState(()=>zp(o)),[g,v]=p.default.useState(()=>Ip(o)),[x,$]=p.default.useState(""),[S,C]=p.default.useState(!1),[_,T]=p.default.useState(!1),[M,O]=p.default.useState(!1),U=p.default.useRef(null),k=p.default.useRef(null),z=p.default.useRef(!1),Z=a||n||S;z.current=Z;let re=p.default.useRef([]),me=p.default.useRef(Promise.resolve());p.default.useEffect(()=>{re.current=g},[g]);let pe=p.default.useRef(null),Ee=p.default.useRef(null),je=p.default.useCallback(()=>{Ee.current&&(window.clearTimeout(Ee.current),Ee.current=null);let P=pe.current;pe.current=null,P&&P.scope===kt()&&qp(P.key,P.text)},[]),bt=p.default.useCallback(()=>{Ee.current&&(window.clearTimeout(Ee.current),Ee.current=null),pe.current=null},[]),pt=p.default.useCallback(()=>{let P=U.current;P&&(P.style.height="auto",P.style.height=`${Math.min(P.scrollHeight,200)}px`)},[]);p.default.useEffect(()=>{pt()},[y,pt]),p.default.useEffect(()=>{if(!u||a)return;let P=window.requestAnimationFrame(()=>{U.current?.focus({preventScroll:!0})});return()=>window.cancelAnimationFrame(P)},[u,a]),p.default.useEffect(()=>(w(zp(o)),()=>je()),[o,je]);let at=p.default.useRef(o);p.default.useEffect(()=>{if(at.current!==o){at.current=o,v(Ip(o)),$("");return}S$(o,g)},[o,g]),p.default.useEffect(()=>{s&&(w(s),window.requestAnimationFrame(()=>{U.current&&(U.current.focus(),U.current.setSelectionRange(s.length,s.length))}))},[s,i]);let Ye=p.default.useCallback(P=>{a||!P||P.length===0||(me.current=me.current.then(async()=>{let{staged:F,errors:B}=await u$(P,{limits:b,existing:re.current,t:m});F.length>0&&v(G=>{let ie=[...G,...F];return re.current=ie,ie}),$(B.length>0?B.join(" "):"")}).catch(()=>{$(m("chat.attachmentStagingFailed"))}))},[a,b,m]),Rn=p.default.useCallback(P=>{v(F=>{let B=F.filter(G=>G.id!==P);return re.current=B,B}),$("")},[]),Da=p.default.useCallback(()=>{a||k.current?.click()},[a]),Cn=p.default.useCallback(P=>{let F=Array.from(P.target.files||[]);Ye(F),P.target.value=""},[Ye]),Ot=p.default.useCallback(async()=>{if(!(!y.trim()||z.current)){z.current=!0,C(!0);try{if(await e(y.trim(),{attachments:g})===null)return;w(""),v([]),re.current=[],$(""),bt(),w$(o),N$(o),U.current&&(U.current.style.height="auto")}catch{}finally{z.current=a||n,C(!1)}}},[y,g,e,o,bt,a,n]),Za=p.default.useCallback(P=>{let F=P.target.value;w(F),pe.current={key:o,text:F,scope:kt()},Ee.current&&window.clearTimeout(Ee.current),Ee.current=window.setTimeout(je,300)},[o,je]),ua=p.default.useCallback(async()=>{if(!(!r||_||!t)){T(!0);try{await t()}finally{T(!1)}}},[r,_,t]),Wa=p.default.useCallback(P=>{if(P.key==="Enter"&&!P.shiftKey){if(P.preventDefault(),U.current?.dataset?.sendDisabled==="true"||z.current)return;Ot()}},[Ot]),en=p.default.useCallback(P=>{let F=Array.from(P.clipboardData?.files||[]);F.length>0&&(P.preventDefault(),Ye(F))},[Ye]),ht=p.default.useCallback(P=>{P.preventDefault(),O(!1);let F=Array.from(P.dataTransfer?.files||[]);F.length>0&&Ye(F)},[Ye]),ca=p.default.useCallback(P=>{P.preventDefault(),!a&&O(!0)},[a]),Vt=p.default.useCallback(P=>{P.currentTarget.contains(P.relatedTarget)||O(!1)},[]),ae=y.trim(),ee=a||n,Te=r&&!ae,Be=m(h?"chat.heroPlaceholder":"chat.followUpPlaceholder"),lt=b.accept.length>0?b.accept.join(","):void 0,N=h?"w-full":"px-4 py-3 sm:px-5 lg:px-8",E=["relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",a?"":"focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",h?"min-h-[120px]":"",a?"opacity-70":""].join(" "),L=["w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6","text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",h?"min-h-[72px]":"min-h-[40px]"].join(" ");return l`
    <div className=${N}>
      <div
        className=${E}
        onDrop=${ht}
        onDragOver=${ca}
        onDragLeave=${Vt}
      >
        ${M&&l`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${m("chat.attachmentDropHint")}
          </div>
        `}
        ${x&&l`
          <div
            role="alert"
            className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
          >
            <span className="min-w-0 flex-1">${x}</span>
            <button
              type="button"
              onClick=${()=>$("")}
              aria-label=${m("common.dismiss")}
              title=${m("common.dismiss")}
              className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
            >
              <${D} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
            </button>
          </div>
        `}

        ${g.length>0&&l`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${g.map(P=>l`
                <div
                  key=${P.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${P.previewUrl?l`<img
                        src=${P.previewUrl}
                        alt=${P.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`:l`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${D} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${P.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${P.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${()=>Rn(P.id)}
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
          onChange=${Za}
          onKeyDown=${Wa}
          onPaste=${en}
          data-send-disabled=${ee?"true":"false"}
          placeholder=${Be}
          rows=${1}
          disabled=${a}
          className=${L}
        />

        <input
          ref=${k}
          type="file"
          multiple
          accept=${lt}
          className="hidden"
          onChange=${Cn}
        />

        <div className="mt-2 flex items-center gap-2">
          ${ee&&l`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${f||m("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick=${Da}
              disabled=${a}
              aria-label=${m("chat.attachFiles")}
              title=${m("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${D} name="plus" className="h-5 w-5" />
            </button>
            ${Te?l`
                <${A}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${ua}
                  disabled=${_}
                  aria-label=${m("common.cancel")}
                  title=${m("common.cancel")}
                  className="rounded-full"
                >
                  <${D} name="close" className="h-5 w-5" />
                <//>
              `:l`
                <${A}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${Ot}
                  disabled=${ee||S||!ae}
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
  `}var D1={connected:"bg-mint/20 text-mint border-mint/30",reconnecting:"bg-copper/20 text-copper border-copper/30",disconnected:"bg-red-500/20 text-red-200 border-red-400/30",connecting:"bg-iron-700/50 text-iron-200 border-iron-700/50",paused:"bg-iron-700/50 text-iron-200 border-iron-700/50",idle:"hidden"};function M1({status:e}){let t=R();if(e==="idle"||e==="connecting"||e==="connected"||!e)return null;let a="connection."+e,n=t(a);return l`
    <div
      className=${["sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",D1[e]||D1.connecting].join(" ")}
    >
      ${n!==a?n:e}
    </div>
  `}function O1({onSuggestion:e,onSend:t,disabled:a,sendDisabled:n,initialText:r,resetKey:s,draftKey:i,autoFocusKey:o,context:u,statusText:c,canCancel:d,onCancel:f}){let m=R(),h=[{icon:"tool",title:m("chat.suggestion1"),detail:m("chat.suggestion1Desc")},{icon:"shield",title:m("chat.suggestion2"),detail:m("chat.suggestion2Desc")},{icon:"plug",title:m("chat.suggestion3"),detail:m("chat.suggestion3Desc")}];return l`
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
        <${Lc}
          onSend=${t}
          disabled=${a}
          sendDisabled=${n}
          initialText=${r}
          resetKey=${s}
          draftKey=${i}
          autoFocusKey=${o}
          variant="hero"
          context=${u}
          statusText=${c}
          canCancel=${d}
          onCancel=${f}
        />
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        ${h.map(b=>l`
            <button
              type="button"
              key=${b.title}
              onClick=${()=>e(b.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-white/10 px-2 py-4 text-left hover:border-signal/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal"
              >
                <${D} name=${b.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-semibold text-iron-100">
                  ${b.title}
                </span>
                <span className="mt-0.5 block text-sm text-iron-300">
                  ${b.detail}
                </span>
              </span>
            </button>
          `)}
      </div>
    </div>
  `}var s4=[{keys:["Enter"],descKey:"shortcuts.send"},{keys:["Shift","Enter"],descKey:"shortcuts.newline"},{keys:["?"],descKey:"shortcuts.help"},{keys:["Esc"],descKey:"shortcuts.close"}];function L1({open:e,onClose:t}){let a=R();return e?l`
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
          ${s4.map((n,r)=>l`
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
  `:null}function U1(e){let t=0,a=0,n=0,r=0,s=0;for(let o of e){if(o.role==="thinking"&&(t+=1),o.role==="tool_activity"){let u=P1([o]);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}if(i4(o)){let u=P1(o.toolCalls);a+=u.tools,n+=u.failed,r+=u.declined,s+=u.running}}let i=[];return t&&i.push(`${t} reasoning`),a&&i.push(`${a} ${a===1?"tool":"tools"}`),n&&i.push(`${n} failed`),r&&i.push(`${r} declined`),!n&&!r&&s&&i.push("running"),{hasError:n>0,hasDeclined:r>0,label:`Activity${i.length?` - ${i.join(", ")}`:""}`}}function P1(e){let t=0,a=0,n=0;for(let r of e)r.toolStatus==="error"&&(t+=1),r.toolStatus==="declined"&&(a+=1),r.toolStatus==="running"&&(n+=1);return{tools:e.length,failed:t,declined:a,running:n}}function i4(e){return e.toolCalls&&e.toolCalls.length>0}var j1=!1;function o4(){j1||!window.DOMPurify||(window.DOMPurify.addHook("afterSanitizeAttributes",e=>{e.tagName==="A"&&e.getAttribute("href")&&(e.setAttribute("target","_blank"),e.setAttribute("rel","noopener noreferrer"))}),j1=!0)}function F1(e){if(!e)return"";if(!window.marked||!window.DOMPurify){let a=document.createElement("div");return a.textContent=e,a.innerHTML}o4();let t=window.marked.parse(e,{gfm:!0,breaks:!0});return window.DOMPurify.sanitize(t)}var oh=360;function l4(e){e&&e.querySelectorAll("pre").forEach(t=>{if(t.dataset.enhanced==="1")return;t.dataset.enhanced="1";let a=t.querySelector("code");if(window.hljs&&a)try{window.hljs.highlightElement(a)}catch{}let n=document.createElement("div");n.className="markdown-code-frame",t.parentNode.insertBefore(n,t),n.appendChild(t);let r=document.createElement("div");r.style.cssText="position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0",n.addEventListener("mouseenter",()=>r.style.opacity="1"),n.addEventListener("mouseleave",()=>r.style.opacity="0");let s=c=>{let d=document.createElement("button");return d.type="button",d.textContent=c,d.style.cssText="font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer",d},i=!1,o=s("Wrap");o.addEventListener("click",()=>{i=!i,t.style.whiteSpace=i?"pre-wrap":"",o.textContent=i?"No wrap":"Wrap"});let u=s("Copy");if(u.addEventListener("click",async()=>{try{await navigator.clipboard.writeText(a?a.innerText:t.innerText),u.textContent="Copied",ai("Code copied",{tone:"success"}),setTimeout(()=>u.textContent="Copy",1400)}catch{}}),r.appendChild(o),r.appendChild(u),n.appendChild(r),t.scrollHeight>oh){t.style.maxHeight=`${oh}px`,t.style.overflowX="auto",t.style.overflowY="hidden";let c=!1,d=document.createElement("button");d.type="button",d.textContent="Show more",d.style.cssText="display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer",d.addEventListener("click",()=>{c=!c,t.style.maxHeight=c?"none":`${oh}px`,t.style.overflowY=c?"visible":"hidden",d.textContent=c?"Show less":"Show more"}),n.appendChild(d)}})}function u4({content:e,className:t=""}){let a=p.default.useRef(null),n=p.default.useMemo(()=>F1(e),[e]);return p.default.useEffect(()=>{l4(a.current)},[n]),l`
    <div
      ref=${a}
      className=${["markdown-body",t].join(" ")}
      dangerouslySetInnerHTML=${{__html:n}}
    />
  `}var ia=p.default.memo(u4);var B1={running:"bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",success:"bg-[var(--v2-positive-text)]",declined:"bg-iron-400",error:"bg-[var(--v2-danger-text)]"},c4={success:"ok",declined:"declined",error:"err",running:"run"},d4=2;function ii({activity:e}){return e.toolCalls&&e.toolCalls.length>0?l`<${f4} tools=${e.toolCalls} />`:l`<${p4} activity=${e} />`}function m4(e,t){let a=0,n=0,r=0,s=0;for(let u of t){let c=String(u.toolName||"").toLowerCase();/(grep|search|find|lookup|query)/.test(c)?n+=1:/(bash|shell|exec|run|command|terminal|spawn|process)/.test(c)?r+=1:/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(c)?a+=1:s+=1}let i=[];a&&i.push(e(a===1?"tool.runFile":"tool.runFiles",{n:a})),n&&i.push(e(n===1?"tool.runSearch":"tool.runSearches",{n})),r&&i.push(e(r===1?"tool.runCommand":"tool.runCommands",{n:r})),s&&i.push(e(s===1?"tool.runOther":"tool.runOthers",{n:s}));let o=i.join(", ");return o.charAt(0).toUpperCase()+o.slice(1)}function f4({tools:e}){let t=R(),a=e.some(o=>o.toolStatus==="error"),n=e.some(o=>o.toolStatus==="error"||o.toolStatus==="declined"),[r,s]=p.default.useState(n);if(p.default.useEffect(()=>{n&&s(!0)},[n]),e.length<=d4)return l`
      <div className="flex flex-col gap-3">
        ${e.map((o,u)=>l`<${ii}
            key=${o.id||o.callId||`${o.toolName}-${u}`}
            activity=${o}
          />`)}
      </div>
    `;let i=m4(t,e);return l`
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

      ${r&&l`
        <div className="mt-2 flex flex-col gap-3">
          ${e.map((o,u)=>l`<${ii}
              key=${o.id||o.callId||`${o.toolName}-${u}`}
              activity=${o}
            />`)}
        </div>
      `}
    </div>
  `}function p4({activity:e,nested:t=!1}){let{toolName:a,toolStatus:n,toolDetail:r,toolError:s,toolDurationMs:i,toolParameters:o,toolResultPreview:u}=e,[c,d]=p.default.useState(n==="error"||n==="declined");p.default.useEffect(()=>{(n==="error"||n==="declined")&&d(!0)},[n]);let f=B1[n]||B1.running,m=i!=null,h=p.default.useId(),b=r||h4(o),y=l`
    <button
      type="button"
      onClick=${()=>d(w=>!w)}
      aria-expanded=${c?"true":"false"}
      aria-controls=${h}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full",f].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${c4[n]||"run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${a}</span
      >
      ${b&&l`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${b}</span
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
        ${y}
        ${c&&l`<${v4}
          controlsId=${h}
          toolDetail=${r}
          toolParameters=${o}
          toolResultPreview=${u}
          toolError=${s}
          toolStatus=${n}
          toolDurationMs=${m?i:null}
        />`}
      </div>
    </div>
  `}function h4(e){return typeof e!="string"?null:e.trim().split(/\r?\n/,1)[0]?.trim()||null}function v4({controlsId:e,toolDetail:t,toolParameters:a,toolResultPreview:n,toolError:r,toolStatus:s,toolDurationMs:i}){let o=R(),u=p.default.useMemo(()=>{let m=[];return r&&m.push({id:s==="declined"?"declined":"error",label:o(s==="declined"?"tool.tabDeclined":"tool.tabError")}),t&&m.push({id:"details",label:o("tool.tabDetails")}),a&&m.push({id:"params",label:o("tool.tabParameters")}),n&&m.push({id:"result",label:o("tool.tabResult")}),m},[o,r,t,a,n,s]),[c,d]=p.default.useState(null),f=c&&u.some(m=>m.id===c)?c:u[0]?.id;return p.default.useEffect(()=>{r&&d(s==="declined"?"declined":"error")},[r,s]),u.length===0?l`
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
        ${f==="result"&&l`<${g4} text=${n} />`}
        ${(f==="error"||f==="declined")&&l`<pre
          className=${["overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono",f==="declined"?"text-iron-300":"text-[var(--v2-danger-text)]"].join(" ")}
        >${r}</pre>`}
      </div>
    </div>
  `}function g4({text:e}){let t=typeof e=="string"?e.trim():"";if(/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(t))return l`<img
      src=${t}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;let a;if((t.startsWith("{")||t.startsWith("["))&&t.length<2e5)try{a=JSON.parse(t)}catch{a=void 0}if(Array.isArray(a)&&a.length>0&&a.every(y4)){let n=Array.from(a.reduce((r,s)=>(Object.keys(s).forEach(i=>r.add(i)),r),new Set));return l`
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
                  >${b4(r[i])}</td>`)}
              </tr>`)}
          </tbody>
        </table>
      </div>
    `}return a!==void 0&&typeof a=="object"?l`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(a,null,2)}</pre>`:l`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${e}</pre>`}function y4(e){return e&&typeof e=="object"&&!Array.isArray(e)&&Object.values(e).every(t=>t===null||typeof t!="object")}function b4(e){return e==null?"":String(e)}function z1({activity:e}){let t=U1(e),a=w4(e),[n,r]=p.default.useState(a);return p.default.useEffect(()=>{a&&r(!0)},[a]),l`
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
            <${x4}
              key=${s.id||`${s.role||"activity"}-${i}`}
              item=${s}
            />
          `)}
        </div>
      `}
    </div>
  `}function x4({item:e}){if(e.role==="thinking")return l`<${$4} content=${e.content} />`;if(e.role==="tool_activity"||lh(e)){let t=lh(e)?{id:e.id,toolCalls:e.toolCalls}:e;return l`<${ii} activity=${t} />`}return null}function $4({content:e}){return e?l`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${D} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${ia} content=${e} className="text-[13px]" />
      </div>
    </div>
  `:null}function lh(e){return e?.toolCalls&&e.toolCalls.length>0}function w4(e){return(e||[]).some(t=>t?.role==="thinking"||t?.toolStatus==="running"||t?.toolStatus==="error"||t?.toolStatus==="declined"?!0:lh(t)?t.toolCalls.some(a=>a?.toolStatus==="running"||a?.toolStatus==="error"||a?.toolStatus==="declined"):!1)}function Pc(e,t){let a=URL.createObjectURL(e);try{let n=document.createElement("a");n.href=a,n.download=t||"download",document.body.appendChild(n),n.click(),n.remove(),setTimeout(()=>URL.revokeObjectURL(a),100)}catch(n){throw URL.revokeObjectURL(a),n}}function S4({att:e}){let t=e.kind==="image"||(e.mime_type||"").toLowerCase().startsWith("image/"),[a,n]=p.default.useState(()=>t&&e.preview_url||null);return p.default.useEffect(()=>{if(!t){n(null);return}if(e.preview_url){n(e.preview_url);return}if(!e.fetch_url){n(null);return}n(null);let r=!1;return hc(e.fetch_url).then(s=>{r||n(s)}).catch(()=>{}),()=>{r=!0}},[t,e.preview_url,e.fetch_url]),t&&a?l`<img
      src=${a}
      alt=${e.filename||"attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`:l`<${D} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`}var q1="flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs",I1="px-3 py-2";function Uc({att:e,onPreview:t,testId:a,dataPath:n,downloadTestId:r}){let[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(e.fetch_url){i(!0);try{let c=await Ca(e.fetch_url);Pc(c,e.filename||"download")}catch{}finally{i(!1)}}},[e.fetch_url,e.filename]),u=l`
    <${S4} att=${e} />
    <span className="truncate">${e.filename||"attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${e.mime_type}${e.size_label?" / "+e.size_label:""}</span
    >
  `;return!e.fetch_url&&!e.preview_url?l`<div
      className=${`${q1} ${I1} items-center gap-2`}
      data-testid=${a}
      data-file-path=${n}
    >
      ${u}
    </div>`:l`<div className=${`${q1} overflow-hidden`}>
    <button
      type="button"
      onClick=${()=>t(e)}
      aria-label=${`Preview ${e.filename||"attachment"}`}
      data-testid=${a}
      data-file-path=${n}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${I1} text-left transition-colors hover:bg-iron-900/80`}
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
  </div>`}var K1={sm:"max-w-sm",md:"max-w-lg",lg:"max-w-2xl",xl:"max-w-4xl",full:"max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]"};function oi({open:e,onClose:t,title:a,size:n="md",className:r="",children:s}){return p.default.useEffect(()=>{if(!e)return;let i=document.body.style.overflow;return document.body.style.overflow="hidden",()=>{document.body.style.overflow=i}},[e]),p.default.useEffect(()=>{if(!e)return;let i=o=>{o.key==="Escape"&&t?.()};return window.addEventListener("keydown",i),()=>window.removeEventListener("keydown",i)},[e,t]),e?l`
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
        className=${V("relative z-10 w-full","bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]","shadow-[0_24px_60px_rgba(0,0,0,0.35)]","rounded-[1.5rem]","flex flex-col max-h-[90dvh] overflow-hidden",K1[n]??K1.md,r)}
      >
        ${a?l`<${uh} onClose=${t}>${a}<//>`:null}
        ${s}
      </div>
    </div>
  `:null}function uh({children:e,onClose:t,className:a=""}){return l`
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
  `}function li({children:e,className:t=""}){return l`
    <div className=${V("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5",t)}>
      ${e}
    </div>
  `}function ui({children:e,className:t=""}){return l`
    <div
      className=${V("shrink-0 flex items-center justify-end gap-3 flex-wrap","px-5 py-4 md:px-7 md:py-5","border-t border-[var(--v2-panel-border)]",t)}
    >
      ${e}
    </div>
  `}var H1=1e5;function jc({attachment:e,onClose:t}){let a=!!e,[n,r]=p.default.useState("loading"),[s,i]=p.default.useState({}),o=e?l$(e.mime_type):"download";if(p.default.useEffect(()=>{if(!e)return;if(r("loading"),i({}),!e.fetch_url&&e.preview_url){i({dataUrl:e.preview_url,downloadUrl:e.preview_url}),r("ready");return}if(!e.fetch_url){r("error");return}let c=!1,d=null;return Ca(e.fetch_url).then(async f=>{d=URL.createObjectURL(f);let m={downloadUrl:d};if(o==="image"||o==="audio"||o==="video")m.dataUrl=await Dp(f);else if(o==="pdf")m.frameUrl=d;else if(o==="text"){let h=await f.text();m.truncated=h.length>H1,m.text=m.truncated?h.slice(0,H1):h}if(c){URL.revokeObjectURL(d);return}i(m),r("ready")}).catch(()=>{c||r("error")}),()=>{c=!0,d&&URL.revokeObjectURL(d)}},[e,o]),!e)return null;let u=e.filename||"attachment";return l`
    <${oi} open=${a} onClose=${t} size="xl">
      <${uh} onClose=${t}>
        <span className="block truncate">${u}</span>
      <//>
      <${li} className="flex min-h-[12rem] items-center justify-center">
        ${n==="loading"&&l`<div className="text-sm text-iron-400">Loading…</div>`}
        ${n==="error"&&l`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${n==="ready"&&l`<${N4} mode=${o} view=${s} filename=${u} />`}
      <//>
      <${ui}>
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
  `}function N4({mode:e,view:t,filename:a}){switch(e){case"image":return l`<img
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
      </div>`}}var _4=/\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;function k4(e){return e.replace(/```[\s\S]*?```/g," ").replace(/`[^`]*`/g," ")}function Q1(e){if(typeof e!="string"||!e)return[];let t=new Set,a=[];for(let n of k4(e).matchAll(_4)){let r=n[0];t.has(r)||(t.add(r),a.push(r))}return a}function V1(e){return e.split("/").filter(Boolean).pop()||e}function G1(e){if(typeof e!="number"||!Number.isFinite(e))return"";if(e<1024)return`${e} B`;let t=["KB","MB","GB"],a=e/1024,n=0;for(;a>=1024&&n<t.length-1;)a/=1024,n+=1;return`${a<10?a.toFixed(1):Math.round(a)} ${t[n]}`}function R4({threadId:e,path:t,onPreview:a}){let[n,r]=p.default.useState({mime_type:"",size_label:""});p.default.useEffect(()=>{let i=!0;return Tx({threadId:e,path:t}).then(o=>{!i||!o?.stat||r({mime_type:o.stat.mime_type||"",size_label:G1(o.stat.size_bytes)})}).catch(()=>{}),()=>{i=!1}},[e,t]);let s={filename:V1(t),mime_type:n.mime_type,size_label:n.size_label,fetch_url:pc({threadId:e,path:t})};return l`<${Uc}
    att=${s}
    onPreview=${a}
    testId="project-file-chip"
    dataPath=${t}
    downloadTestId="project-file-download"
  />`}function Y1({threadId:e,content:t}){let a=p.default.useMemo(()=>Q1(t),[t]),[n,r]=p.default.useState(null);return!e||a.length===0?null:l`
    <div className="mt-2 flex flex-col gap-1.5">
      ${a.map(s=>l`<${R4}
          key=${s}
          threadId=${e}
          path=${s}
          onPreview=${r}
        />`)}
      <${jc}
        attachment=${n}
        onClose=${()=>r(null)}
      />
    </div>
  `}var J1={user:"ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",assistant:"mr-auto px-1 text-iron-100",system:"mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",error:"mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200"};function C4(e){if(!e)return"";let t=new Date(e);return Number.isNaN(t.getTime())?"":t.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}function E4({content:e}){let[t,a]=p.default.useState(!1);return e?l`
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
          <${ia} content=${e} className="text-[13px]" />
        </div>
      `}
    </div>
  `:null}function T4({message:e,onRetry:t,threadId:a}){let{role:n,content:r,images:s,attachments:i,generatedImages:o,isOptimistic:u,status:c,error:d,toolCalls:f,timestamp:m}=e,h=n==="user",[b,y]=p.default.useState(!1),[w,g]=p.default.useState(null),v=p.default.useCallback(async()=>{try{await navigator.clipboard.writeText(typeof r=="string"?r:""),y(!0),ai("Copied to clipboard",{tone:"success"}),setTimeout(()=>y(!1),1400)}catch{}},[r]);if(n==="tool_activity"||f&&f.length>0){let U=f&&f.length>0?{id:e.id,toolCalls:f}:e;return l`<${ii} activity=${U} />`}if(n==="thinking")return l`<${E4} content=${r} />`;if(n==="image")return l`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${(o||[]).map((k,z)=>k.data_url?l`<img key=${z} src=${k.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`:l`
                  <div key=${z} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${k.path&&l`<div className="mt-1 font-mono text-xs text-iron-300">${k.path}</div>`}
                  </div>
                `)}
        </div>
      </div>
    `;let x=C4(m),$=n==="user"||n==="assistant"&&!u,S=n==="system"||n==="error",C=h?"max-w-[85%]":S?"mx-auto max-w-[85%]":"w-full max-w-[85%]",_=h?"":"w-full min-w-0 max-w-full",T=c==="error"&&t,M=h&&c==="queued",O=$||T||x;return l`
    <div
      data-testid=${`msg-${n}`}
      className=${["group flex w-full min-w-0 flex-col",h?"items-end":"items-start"].join(" ")}
    >
      <div className=${["flex min-w-0 flex-col",C].join(" ")}>
        <div
          className=${["text-base leading-7",_,J1[n]||J1.assistant,u?"opacity-70":""].join(" ")}
        >
          ${n==="assistant"||n==="system"||n==="error"?l`<${ia} content=${r} />`:l`<div className="whitespace-pre-wrap">${r}</div>`}

          ${c==="error"&&l`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${d}</span>
            </div>
          `}

          ${M&&l`
            <div className="mt-2 flex justify-end">
              <span className="rounded border border-iron-600 bg-iron-900/70 px-1.5 py-0.5 text-[11px] font-medium uppercase tracking-normal text-iron-300">
                Queued
              </span>
            </div>
          `}

          ${s&&s.length>0&&l`
            <div className="mt-2 flex flex-wrap gap-2">
              ${s.map((U,k)=>l`<img key=${k} src=${U} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${i&&i.length>0&&l`
            <div className="mt-2 flex flex-col gap-1.5">
              ${i.map((U,k)=>l`<${Uc}
                key=${U.id||k}
                att=${U}
                onPreview=${g}
              />`)}
            </div>
            <${jc}
              attachment=${w}
              onClose=${()=>g(null)}
            />
          `}

          ${n==="assistant"&&l`<${Y1}
            threadId=${a}
            content=${typeof r=="string"?r:""}
          />`}
        </div>
      </div>

      ${O&&l`
        <div
          className=${["mt-1 flex min-h-7 w-max max-w-[85%] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",h?"self-end justify-end":S?"self-center justify-center":"self-start justify-start"].join(" ")}
        >
          ${x&&l`<time dateTime=${m} className="shrink-0 font-mono text-[11px] text-iron-500">${x}</time>`}
          ${($||T)&&l`
            <div className="flex shrink-0 items-center gap-1">
            ${$&&l`
              <button
                type="button"
                onClick=${v}
                title=${b?"Copied":"Copy message"}
                aria-label=${b?"Copied":"Copy message"}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-iron-100"
              >
                <${D} name=${b?"check":"copy"} className="h-3.5 w-3.5" />
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
                <${D} name="retry" className="h-3.5 w-3.5" />
              </button>
            `}
            </div>
          `}
        </div>
      `}
    </div>
  `}var X1=p.default.memo(T4);function n2(e){let t=A4(e),a=[];for(let n=0;n<t.length;n+=1){let r=t[n];if(r2(r)){let s=Z1(t,n+1),i=t[n+1+s.length];if(s.length>0&&(!i||i.role==="user")){W1(a,s),e2(a,r),n+=s.length;continue}}if(ch(r)){let s=Z1(t,n);W1(a,s),n+=s.length-1;continue}e2(a,r)}return a}function A4(e){let t=new Map;for(let s=0;s<e.length;s+=1){let i=e[s],o=Fc(i);o&&r2(i)&&t.set(o,s)}if(t.size===0)return e;let a=new Map,n=new Set;for(let s=0;s<e.length;s+=1){let i=e[s];if(!ch(i))continue;let o=Fc(i),u=o?t.get(o):void 0;if(u===void 0||u>=s)continue;let c=a.get(u)||[];c.push(i),a.set(u,c),n.add(s)}if(n.size===0)return e;let r=[];for(let s=0;s<e.length;s+=1){if(n.has(s))continue;let i=a.get(s);i&&r.push(...i),r.push(e[s])}return r}function Z1(e,t){let a=t,n=Fc(e[t]);for(;a<e.length&&ch(e[a])&&D4(n,e[a]);)a+=1;return e.slice(t,a)}function D4(e,t){let a=Fc(t);return!e||!a||a===e}function W1(e,t){if(t.length===0)return;let a=M4(t);e.push({type:"activity-run",id:`activity-run-${a[0].id}`,activity:a})}function e2(e,t){e.push({type:"message",id:t.id,message:t})}function r2(e){return e.role==="assistant"&&!s2(e)&&(e.isFinalReply===!0||(e.kind==="assistant"||e.kind==="assistant_message")&&e.status==="finalized")}function ch(e){return e.role==="thinking"||e.role==="tool_activity"||s2(e)}function s2(e){return e?.toolCalls&&e.toolCalls.length>0}function Fc(e){return e?.turnRunId||null}function M4(e){return[...e].sort((t,a)=>t?.role!=="tool_activity"||a?.role!=="tool_activity"?0:O4(t,a))}function O4(e,t){if(Number.isFinite(e.activityOrder)&&Number.isFinite(t.activityOrder)){let n=e.activityOrder-t.activityOrder;if(n!==0)return n}let a=t2(a2(e.updatedAt||e.timestamp),a2(t.updatedAt||t.timestamp));return a!==0?a:t2(e.sequence,t.sequence)}function t2(e,t){let a=Number.isFinite(e)?e:null,n=Number.isFinite(t)?t:null;return a===null&&n===null?0:a===null?1:n===null?-1:a-n}function a2(e){if(!e)return null;let t=Date.parse(e);return Number.isFinite(t)?t:null}var L4=100,P4=100;function U4(e){return e?e.scrollHeight-e.scrollTop-e.clientHeight:Number.POSITIVE_INFINITY}function i2(e,t=L4){return U4(e)<=t}function o2(e){e&&(e.scrollTop=Math.max(0,e.scrollHeight-e.clientHeight))}function l2(e){return e?.id?`${e.role||""}:${e.id}`:null}function j4(e,t){let a=l2(t);return!!(a&&t?.role==="user"&&a!==e)}function u2({messages:e,isLoading:t,hasMore:a,onLoadMore:n,onRetryMessage:r,threadId:s,pending:i=!1,children:o}){let u=R(),c=p.default.useRef(null),d=p.default.useRef(null),f=p.default.useRef(!0),m=p.default.useRef(null),h=p.default.useRef(null),b=p.default.useRef(null),y=p.default.useRef(0),w=p.default.useRef(!1),[g,v]=p.default.useState(!0),x=p.default.useCallback(()=>{h.current!==null&&(window.cancelAnimationFrame(h.current),h.current=null)},[]),$=p.default.useCallback((k=!1)=>{c.current&&(k&&(f.current=!0,w.current=!1),f.current&&(x(),h.current=window.requestAnimationFrame(()=>{h.current=null;let Z=c.current;!Z||!k&&!f.current||(o2(Z),y.current=Z.scrollTop,w.current=!1,v(!0))})))},[x]),S=p.default.useCallback(()=>{b.current!==null&&(window.cancelAnimationFrame(b.current),b.current=null)},[]);p.default.useLayoutEffect(()=>{let k=e.length>0?e[e.length-1]:null,z=l2(k),Z=j4(m.current,k);return m.current=z,$(Z),x},[e,i,$,x]),p.default.useLayoutEffect(()=>{let k=d.current;if(!k||typeof ResizeObserver!="function")return;let z=new ResizeObserver(()=>{$()});return z.observe(k),()=>{z.disconnect(),x()}},[$,x]);let C=p.default.useCallback(()=>{b.current=null;let k=c.current;if(!k)return;let z=i2(k);y.current=k.scrollTop,z?(f.current=!0,w.current=!1,v(!0)):w.current?(f.current=!1,v(!1)):(f.current=!0,v(!0),$()),a&&k.scrollTop<P4&&n&&!t&&n()},[a,n,t,$]),_=p.default.useCallback(()=>{w.current=!0},[]),T=p.default.useCallback(k=>{let z=c.current;if(!z||typeof k?.clientX!="number")return;let Z=z.offsetWidth-z.clientWidth;if(Z<=0)return;let re=z.getBoundingClientRect().right;k.clientX>=re-Z-2&&(w.current=!0)},[]),M=p.default.useCallback(()=>{let k=c.current;if(!k)return;let z=i2(k),Z=k.scrollTop<y.current;y.current=k.scrollTop,!z&&Z&&(w.current=!0),z?(f.current=!0,w.current=!1):w.current?(f.current=!1,x()):f.current=!0,b.current===null&&(b.current=window.requestAnimationFrame(C))},[x,C]),O=p.default.useCallback(()=>{let k=c.current;k&&(o2(k),y.current=k.scrollTop,f.current=!0,w.current=!1,v(!0))},[]);p.default.useEffect(()=>S,[S]);let U=p.default.useMemo(()=>n2(e),[e]);return l`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${c}
      onScroll=${M}
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
        ${U.map(k=>k.type==="activity-run"?l`<${z1} key=${k.id} activity=${k.activity} />`:l`<${X1}
                key=${k.id}
                message=${k.message}
                onRetry=${r}
                threadId=${s}
              />`)}
        ${o}
      </div>
    </div>
    ${!g&&l`
      <button
        type="button"
        onClick=${O}
        aria-label=${u("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${D} name="arrowDown" className="h-3.5 w-3.5" />
        ${u("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `}function c2({notice:e,onRecover:t}){return l`
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
  `}function d2({suggestions:e,onSelect:t,disabled:a=!1}){return!e||e.length===0?null:l`
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
  `}function m2(){return l`
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
  `}function Bc(){return H("/api/webchat/v2/channels/connectable")}function f2(e,t){if(!dh(e))return null;let a=zc(e),n=q4(a),r=null;for(let s of t||[]){if(!z4(s))continue;let i=I4(a,s,{commandAliasesOnly:n});i>(r?.matchLength||0)&&(r={channel:s,matchLength:i})}return r?.channel||null}function dh(e){let t=zc(e);if(!t)return!1;let a=/(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(t),n=/(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(t);return a&&n}function F4(e){return[e?.channel,e?.display_name,...Array.isArray(e?.command_aliases)?e.command_aliases:[]].filter(Boolean)}function B4(e,t={}){let a=Array.isArray(e?.command_aliases)?e.command_aliases.filter(Boolean):[];return t.channelManagementOnly?a.filter(n=>p2(zc(n))):a}function z4(e){return e?.strategy!=="admin_managed_channels"}function q4(e){return h2(e,"slack")&&p2(e)}function p2(e){return/(^|\s)(channel|channels|allowlist)(\s|$)/.test(e)}function zc(e){return String(e||"").toLowerCase().replace(/[^a-z0-9]+/g," ").trim().replace(/\s+/g," ")}function I4(e,t,a={}){return(a.commandAliasesOnly?B4(t,{channelManagementOnly:!0}):F4(t)).reduce((r,s)=>{let i=zc(s);return h2(e,i)?Math.max(r,i.length):r},0)}function h2(e,t){return t?` ${e} `.includes(` ${t} `):!1}function g2(e,t){if(!t)return null;if(e==="gate"){let a=t.approval_context||null,n={kind:"gate",gateKind:"approval",runId:t.turn_run_id,gateRef:t.gate_ref,invocationId:t.invocation_id||null,headline:t.headline,body:t.body,allowAlways:t.allow_always===!0};return b2(n,a,t.body)}return e==="auth_required"?{kind:"auth_required",gateKind:"auth",challengeKind:t.challenge_kind||(t.provider||t.account_label||t.authorization_url||t.expires_at?"other":"manual_token"),runId:t.turn_run_id,gateRef:t.auth_request_ref,invocationId:t.invocation_id||null,provider:t.provider||null,accountLabel:t.account_label||"",authorizationUrl:t.authorization_url||null,expiresAt:t.expires_at||null,headline:t.headline,body:t.body}:null}function y2(e){if(!e?.run_id||!e.gate_ref)return null;let t=e.gate_kind||"generic",a={gateKind:t,runId:e.run_id,gateRef:e.gate_ref,invocationId:e.invocation_id||null,headline:e.headline,body:e.body||"",allowAlways:e.allow_always===!0};if(t==="auth"){let n=e.auth_context||{};return{...a,kind:"auth_required",challengeKind:n.challenge_kind||"other",provider:n.provider||null,accountLabel:n.account_label||"",authorizationUrl:n.authorization_url||null,expiresAt:n.expires_at||null}}return b2({...a,kind:"gate"},e.approval_context||null,a.body)}function b2(e,t,a){if(!t){let r=v2(a);return r?{...e,description:r}:e}let n=K4(t);return{...e,toolName:t.tool_name||null,description:t.reason||v2(a),actionLabel:t.action?.label||null,destination:t.destination||null,approvalScope:t.scope||null,approvalDetails:n,parameters:n.length?n.map(r=>`${r.label}: ${r.value}`).join(`
`):null}}function v2(e){return typeof e=="string"&&e.trim()?e.trim():null}function K4(e){let t=[];e.action?.label&&t.push({label:"Action",value:e.action.label}),e.destination?.label&&t.push({label:"Destination",value:e.destination.label}),e.scope?.label&&t.push({label:"Scope",value:e.scope.label});for(let a of e.details||[])!a?.label||a.value==null||t.push({label:a.label,value:String(a.value)});return t}function x2({status:e,failureCategory:t,failureSummary:a}){return typeof a=="string"&&a.trim()?a.trim():typeof t=="string"&&t.trim()?`The run failed: ${t.trim().replaceAll("_"," ")}.`:e==="recovery_required"?"The run is awaiting recovery \u2014 backend reported `recovery_required`.":"The run failed before producing a reply."}function $2(){return{terminalByInvocation:new Map}}function w2(e){e?.current?.terminalByInvocation?.clear()}function fh(e,t,a){let n=N2(t,{toolStatus:"running"});n&&ci(e,n,a)}function S2(e,t,a,n="gate_declined"){let r=N2(t,{toolStatus:"declined",toolError:n,toolErrorKind:"gate_declined"});r&&ci(e,r,a)}function ci(e,t,a){if(!t)return;let n=X4(t);n=J4(n,a),e(r=>{let s=_2(n),i=V4(r,n,s);if(i>=0){let u=[...r];return u[i]=G4(u[i],n),mh(u[i],a),u}let o={id:s,role:"tool_activity",...n};return mh(o,a),[...r,o]})}function N2(e,t={}){let a=e?.kind==="gate",n=e?.kind==="auth_required",r=n&&t.toolStatus==="declined";if(!e?.runId||!e?.gateRef||!e.invocationId&&!r||!a&&!n)return null;let s=e.invocationId||Q4(e),i=e.toolName||e.headline||e.gateKind||"gate";return{invocationId:s,callId:s,capabilityId:e.toolName||e.gateKind||null,toolName:Ho(i)||i,toolStatus:t.toolStatus||"running",toolDetail:e.actionLabel||null,toolParameters:H4(e),toolResultPreview:null,toolError:t.toolError||null,toolErrorKind:t.toolErrorKind||null,toolDurationMs:null,updatedAt:t.updatedAt||new Date().toISOString(),resultRef:null,truncated:!1,outputBytes:null,outputKind:null,turnRunId:e.runId,gateRef:e.gateRef,gateActivity:!0}}function H4(e){if(typeof e?.parameters=="string"&&e.parameters.trim())return e.parameters.trim();if(!Array.isArray(e?.approvalDetails)||e.approvalDetails.length===0)return null;let t=e.approvalDetails.filter(a=>a?.label&&a.value!=null).map(a=>`${a.label}: ${a.value}`);return t.length>0?t.join(`
`):null}function Q4(e){return`gate:${e.runId}:${e.kind}:${e.gateRef}`}function _2(e){return`tool-${e.invocationId}`}function V4(e,t,a){let n=e.findIndex(s=>s?.id===a);if(n>=0)return n;let r=t.gateRef||null;if(r){let s=e.findIndex(i=>i?.role==="tool_activity"&&i.turnRunId===t.turnRunId&&i.gateRef===r);if(s>=0)return s}return-1}function G4(e,t){let a=Ko(e.toolStatus),n=Ko(t.toolStatus),r=a&&!n,s=t.gateActivity&&!e.gateActivity,i={...e,...t,id:e.id,role:"tool_activity",invocationId:e.gateActivity&&!t.gateActivity?t.invocationId:e.invocationId||t.invocationId,callId:e.gateActivity&&!t.gateActivity?t.callId:e.callId||t.callId,toolName:s?e.toolName:t.toolName||e.toolName,toolStatus:r?e.toolStatus:t.toolStatus,toolDetail:t.toolDetail||e.toolDetail||null,toolParameters:t.toolParameters||e.toolParameters||null,toolResultPreview:t.toolResultPreview||e.toolResultPreview||null,toolError:t.toolError||e.toolError,toolErrorKind:t.toolErrorKind||e.toolErrorKind||null,updatedAt:r?e.updatedAt||t.updatedAt:t.updatedAt||e.updatedAt,turnRunId:t.turnRunId||e.turnRunId||null,gateRef:t.gateRef||e.gateRef||null,gateActivity:e.gateActivity&&t.gateActivity,capabilityId:s?e.capabilityId||t.capabilityId||null:t.capabilityId||e.capabilityId||null,activityOrder:Y4(e,t),activityOrderSource:t.activityOrderSource||e.activityOrderSource||null};return e.gateActivity&&!t.gateActivity&&(i.id=_2(t),i.gateActivity=!1),i}function Y4(e,t){return Number.isFinite(t.activityOrder)?t.activityOrder:e.activityOrder}function J4(e,t){if(!e?.invocationId)return e;if(Ko(e.toolStatus))return mh(e,t),e;let a=t?.current?.terminalByInvocation?.get(e.invocationId);return a?Number.isFinite(e.activityOrder)?{...a,activityOrder:e.activityOrder,activityOrderSource:e.activityOrderSource||a.activityOrderSource||null}:a:e}function mh(e,t){!e?.invocationId||!Ko(e.toolStatus)||t?.current?.terminalByInvocation?.set(e.invocationId,e)}function X4(e){let t=Ho(e.toolName||e.capabilityId);return{...e,toolName:t||e.toolName||"tool"}}function T2({threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o,onRunSettled:u}){let c=p.default.useRef(new Set),d=p.default.useRef(null),f=p.default.useRef(null);return p.default.useCallback(m=>{let{type:h,frame:b}=m||{};if(!(!h||!b))switch(h){case"accepted":{let y=b.ack||{};y.run_id&&(d.current=y.run_id),r?.({runId:y.run_id||null,threadId:y.thread_id||e,status:y.status||null}),a(!0);return}case"running":case"capability_progress":{let y=b.progress||{};y.turn_run_id&&(d.current=y.turn_run_id,r?.(w=>w&&w.runId===y.turn_run_id?{...w,status:"running"}:{runId:y.turn_run_id,threadId:e,status:"running"}),Z4(n,y.turn_run_id,f)),a(!0);return}case"capability_activity":{let y=b.activity;if(!y||!y.invocation_id)return;ci(t,Fp(y),o);return}case"capability_display_preview":{let y=b.preview;if(!y||!y.invocation_id)return;let w=jp(y);ci(t,w,o);return}case"gate":case"auth_required":{let y=g2(h,b.prompt);y&&(fh(t,y,o),n(y),r?.({runId:y.runId,threadId:e,status:"awaiting_gate"})),a(!1);return}case"final_reply":{let y=b.reply||{};t(w=>[...w,{id:`reply-${y.turn_run_id||Date.now()}`,role:"assistant",content:y.text||"",timestamp:y.generated_at||new Date().toISOString(),turnRunId:y.turn_run_id,isFinalReply:!0}]),n(null),a(!1);return}case"cancelled":{let y=b.run_state?.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),Kc(c,u,y,!1);return}case"failed":{let y=b.run_state||{},w=y.run_id||s?.current?.runId||null;n(null),a(!1),r?.(null),hh(t,{runId:w,status:y.status||"failed",failureCategory:a5(y),failureSummary:null}),Kc(c,u,w,!1);return}case"projection_snapshot":case"projection_update":{let y=b.state?.items||[];e5({items:y,threadId:e,setMessages:t,setIsProcessing:a,setPendingGate:n,setActiveRun:r,onRunSettled:u,settledRunsRef:c,latestRunIdRef:d,promptRunIdRef:f,activeRunRef:s,locallyResolvedGatesRef:i,toolActivityStateRef:o});return}case"keep_alive":default:return}},[e,t,a,n,r,s,i,o,u])}function Kc(e,t,a,n){!t||!a||!e?.current||e.current.has(a)||(e.current.add(a),t(a,{success:n}))}var k2=new Set(["completed","succeeded","failed","cancelled","recovery_required"]),R2=new Set(["completed","succeeded"]),qc=new Set(["blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]),Ic=new Set(["awaiting_gate","blocked_auth","blocked_approval","blocked_resource","blocked_dependent_run"]);function C2(e,t,a){t&&(a?.current===t&&(a.current=null),e(n=>n?.runId===t?null:n))}function Z4(e,t,a){t&&e(n=>n?.runId!==t||n.kind==="auth_required"?n:(a?.current===t&&(a.current=null),null))}function W4(e,t,a,n,r,s){let i=t?.runId||null;if(!i||r?.has(i))return!0;let o=a?.get(i);if(o)return!Ic.has(o);let u=e?.current,c=u?.runId||n?.current||null;if(c&&i!==c)return!0;let d=s?.current===c;return c&&i===c&&!d&&u?.status&&!Ic.has(u.status)?!0:!u?.runId||!u.status?!1:!Ic.has(u.status)}function e5({items:e,threadId:t,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,activeRunRef:d,locallyResolvedGatesRef:f,toolActivityStateRef:m}){let h=new Map,b=new Set,y=d?.current||null,w=y?.runId||u?.current||null;for(let v of e){let x=v.run_status;x?.run_id&&x.status&&(h.set(x.run_id,x.status),w&&w!==x.run_id&&y?.status&&!k2.has(y.status)&&qc.has(x.status)&&b.add(x.run_id))}let g=u?.current??null;for(let v of e){if(v.run_status){let{run_id:x,status:$,failure_category:S,failure_summary:C}=v.run_status,_=k2.has($),T=d?.current?.source==="local"?d.current.runId:null,M=!!(x&&T&&T!==x),O=g??u?.current??null,U=!!(_&&x&&O&&O!==x),k=x&&qc.has($)?E2(f,x):null;if(x&&b.has(x)||M)continue;if(U){E2(f,d?.current?.runId)?.outcome==="resumed"&&(t5({runId:x,activePromptRunId:d?.current?.runId,success:R2.has($),status:$,failureCategory:S,failureSummary:C,setMessages:a,setIsProcessing:n,setPendingGate:r,setActiveRun:s,onRunSettled:i,settledRunsRef:o,latestRunIdRef:u,promptRunIdRef:c,locallyResolvedGatesRef:f}),g=null);continue}if(k){C2(r,x,c),k.outcome==="resumed"?(n(!0),s?.(z=>z&&z.runId===x?{...z,status:z.status==="awaiting_gate"?"queued":z.status||"queued"}:{runId:x,threadId:t,status:"queued"}),g=x,u&&(u.current=x)):(n(!1),d?.current?.runId===x&&s?.(null),g=null,u?.current===x&&(u.current=null));continue}x&&(g=x,!_&&u&&(u.current=x),s?.(z=>z&&z.runId===x?{...z,status:$}:{runId:x,threadId:t,status:$})),x&&qc.has($)?c&&(c.current=x):x&&c?.current===x&&(c.current=null),_?(n(!1),r(null),s?.(null),ph(f,x),g=null,u&&(u.current=null),x&&c?.current===x&&(c.current=null),Kc(o,i,x,R2.has($)),($==="failed"||$==="recovery_required")&&hh(a,{runId:x,status:$,failureCategory:S,failureSummary:C})):qc.has($)||(C2(r,x,c),ph(f,x),n(!0))}if(v.text){let x=`text-${v.text.id}`;a($=>{let S=$.findIndex(_=>_.id===x),C={id:x,role:"assistant",content:v.text.body||"",timestamp:new Date().toISOString(),isFinalReply:!0};if(S>=0){let _=[...$];return _[S]=C,_}return[...$,C]}),n(!1)}if(v.thinking){let x=`thinking-${v.thinking.id}`;a($=>{let S=$.findIndex(_=>_.id===x),C={id:x,role:"thinking",content:v.thinking.body||"",timestamp:new Date().toISOString(),turnRunId:v.thinking.run_id||null};if(S>=0){let _=[...$];return _[S]=C,_}return[...$,C]})}if(v.capability_activity){let x=v.capability_activity;x.invocation_id&&ci(a,Fp(x),m)}if(v.gate){let x=y2(v.gate),$=x?.runId||null;$&&!W4(d,x,h,u,b,c)&&!r5(f,$,x.gateRef)&&(fh(a,x,m),r(S=>S||x),s?.(S=>S&&S.runId===$?{...S,status:Ic.has(S.status)?S.status:"awaiting_gate"}:{runId:$,threadId:t,status:"awaiting_gate"}),c&&(c.current=$),n(!1))}if(v.skill_activation){let{id:x,skill_names:$=[],feedback:S=[]}=v.skill_activation;if($.length||S.length){let C=`skill-${x||$.join("-")||"activation"}`,_=[$.length?`Skill activated: ${$.join(", ")}`:"",...S].filter(Boolean).join(`
`);a(T=>T.some(M=>M.id===C)?T:[...T,{id:C,role:"system",content:_,timestamp:new Date().toISOString()}])}}}u&&g&&(u.current=g)}function t5({runId:e,activePromptRunId:t,success:a,status:n,failureCategory:r,failureSummary:s,setMessages:i,setIsProcessing:o,setPendingGate:u,setActiveRun:c,onRunSettled:d,settledRunsRef:f,latestRunIdRef:m,promptRunIdRef:h,locallyResolvedGatesRef:b}){o(!1),u(null),c?.(null),ph(b,t),m&&(m.current=null),h?.current===t&&(h.current=null),Kc(f,d,e,a),(n==="failed"||n==="recovery_required")&&hh(i,{runId:e,status:n,failureCategory:r,failureSummary:s})}function a5(e){let t=e?.failure;return typeof t=="string"&&t.trim()?t.trim():t&&typeof t=="object"&&typeof t.category=="string"&&t.category.trim()?t.category.trim():null}function hh(e,{runId:t,status:a,failureCategory:n,failureSummary:r}){let s=`err-${t||"unknown"}`;e(i=>{let o=i.findIndex(c=>c.id===s),u=x2({status:a,failureCategory:n,failureSummary:r});if(o>=0){if(!r||i[o].content===u)return i;let c=[...i];return c[o]={...c[o],content:u},c}return[...i,{id:s,role:"error",content:u,timestamp:new Date().toISOString(),turnRunId:t||null}]})}function E2(e,t){if(!t)return null;let a=e?.current;if(!a)return null;for(let[n,r]of a.entries())if(n.startsWith(`${t}
`))return n5(r);return null}function n5(e){return e&&typeof e=="object"?{resolution:e.resolution||null,outcome:e.outcome||null}:{resolution:e||null,outcome:null}}function ph(e,t){if(!t)return;let a=e?.current;if(a)for(let n of Array.from(a.keys()))n.startsWith(`${t}
`)&&a.delete(n)}function r5(e,t,a){return!t||!a?!1:!!e?.current?.has(`${t}
${a}`)}function A2(e,t,a){let n=e.get(t)||[];e.set(t,[...n,a])}function D2(e,t,a){let n=(e.get(t)||[]).filter(r=>r.id!==a);n.length>0?e.set(t,n):e.delete(t)}function M2(e,t,a,n){let r=vh(n);return r?(s5(e,t,a,{timelineMessageId:r}),r):null}function s5(e,t,a,n){let s=(e.get(t)||[]).map(i=>i.id===a?{...i,...n}:i);s.length>0&&e.set(t,s)}function vh(e){return typeof e!="string"?null:e.startsWith("msg:")?e.slice(4):null}var i5=["accepted","running","capability_progress","capability_activity","capability_display_preview","gate","auth_required","final_reply","cancelled","failed","projection_snapshot","projection_update","keep_alive","error"];function O2({threadId:e,onEvent:t,enabled:a}){let[n,r]=p.default.useState("idle"),s=p.default.useRef(t);s.current=t;let i=p.default.useRef(null);return p.default.useEffect(()=>{if(!a||!e){r("idle");return}i.current=null;let o=null,u=null,c=0,d=3e4;function f(){if(document.visibilityState==="hidden"){r("paused");return}r(c>0?"reconnecting":"connecting"),o=Hx({threadId:e,afterCursor:i.current||void 0}),o.onopen=()=>{c=0,r("connected")},o.onerror=()=>{o&&o.close(),r("disconnected"),c++;let y=Math.min(1e3*2**c,d);u=setTimeout(f,y)};let b=(y,w)=>{let g=null;try{g=JSON.parse(y.data)}catch{return}!g||typeof g!="object"||(y.lastEventId&&(i.current=y.lastEventId),s.current?.({type:g.type||w,frame:g,lastEventId:y.lastEventId||null}))};o.onmessage=y=>b(y,"message");for(let y of i5)o.addEventListener(y,w=>b(w,y))}function m(){u&&(clearTimeout(u),u=null),o&&(o.close(),o=null),r("paused")}function h(){document.visibilityState==="hidden"?m():o||f()}return f(),document.addEventListener("visibilitychange",h),()=>{document.removeEventListener("visibilitychange",h),u&&clearTimeout(u),o&&o.close()}},[a,e]),{status:n}}var o5=3e4,l5="credential_stored_gate_resolution_failed",u5="approval_gate_pending_send_blocked",c5="ironclaw-product-auth",gh="ironclaw:product-auth:oauth-complete",d5="ironclaw:product-auth:oauth-complete";async function L2(e){let t=new AbortController,a=setTimeout(()=>t.abort(),o5);try{return await e(t.signal)}finally{clearTimeout(a)}}function m5(e){let t=new Error("auth gate resolution failed after credential storage");return t.safeAuthGateCode=l5,t.cause=e,t}function f5(){let e=new Error("Resolve the approval request before sending another message.");return e.safeErrorCode=u5,e}function p5(e){let a=At.getQueryData?.(["threads"])?.threads;return Array.isArray(a)?!a.find(r=>r.thread_id===e||r.id===e)?.title:!0}function P2(e,t){return!e||!t?.runId||!t?.gateRef?null:`${e}
${t.runId}
${t.gateRef}`}function h5(e){return e?.continuation?.type==="turn_gate_resume"}function v5(e){if(e?.outcome)return e.outcome;let t=String(e?.status||"").toLowerCase();return t==="queued"||t==="running"?"resumed":t==="cancelled"||e?.already_terminal===!0?"cancelled":e?.already_terminal===!1?"resumed":null}function U2(e){return e?.kind==="auth_required"&&e?.challengeKind==="oauth_url"}function g5(e){return e?.type===d5&&e?.status==="completed"}function y5(e,t,a){if(!g5(e))return!1;let n=e?.continuation;return!n||n.type!=="turn_gate_resume"?Number(e?.completedAt||0)>=a:!(n.turn_run_ref&&n.turn_run_ref!==t?.runId||n.gate_ref&&n.gate_ref!==t?.gateRef)}function yh(e){if(!e)return null;try{return JSON.parse(e)}catch{return null}}async function b5(e){if(!dh(e))return null;try{let a=(await At.fetchQuery({queryKey:["connectable-channels"],queryFn:Bc}))?.channels||[];return f2(e,a)}catch(t){return console.error("Failed to resolve connectable channels:",t),null}}function j2(e){let t=p.default.useRef(e),a=p.default.useRef(new Map),n=p.default.useRef(1),[r,s]=p.default.useState(0),[i,o]=p.default.useState(Date.now()),[u,c]=p.default.useState(null),d=p.default.useRef(u),f=p.default.useCallback(ae=>{let ee=typeof ae=="function"?ae(d.current):ae;d.current=ee,c(ee)},[]);p.default.useEffect(()=>{d.current=u},[u]);let[m,h]=p.default.useState(null),b=p.default.useCallback(()=>a.current.get(e||"__new__")||[],[e]),y=p.default.useCallback(ae=>{let ee=e||"__new__";ae.length>0?a.current.set(ee,ae):a.current.delete(ee)},[e]),{messages:w,hasMore:g,nextCursor:v,isLoading:x,loadError:$,loadHistory:S,seedThreadMessages:C,setMessages:_}=x$(e,{getPendingMessages:b,setPendingMessages:y}),[T,M]=p.default.useState(!1),O=p.default.useRef(T),U=p.default.useCallback(ae=>{let ee=typeof ae=="function"?ae(O.current):ae;O.current=ee,M(ee)},[]),[k,z]=p.default.useState(null),Z=p.default.useRef(k),[re,me]=p.default.useState(null),pe=p.default.useCallback(ae=>{let ee=Z.current,Te=typeof ae=="function"?ae(ee):ae;Object.is(Te,ee)||(Z.current=Te,z(Te))},[]),[Ee,je]=p.default.useState(e),bt=p.default.useRef($2()),pt=p.default.useRef(new Map),at=p.default.useRef({gateKey:null,credentialRef:null,inFlight:!1}),Ye=p.default.useRef(!1);Ee!==e&&(je(e),M(!1),z(null),me(null),c(null),h(null)),p.default.useEffect(()=>{t.current=e},[e]),p.default.useEffect(()=>{Z.current=k},[k]),p.default.useEffect(()=>{O.current=T},[T]),p.default.useEffect(()=>{let ae=P2(e,k);me(ee=>ee&&ee.gateKey!==ae?null:ee)},[k,e]),p.default.useEffect(()=>{w2(bt),pt.current.clear()},[e]);let Rn=Math.max(0,Math.ceil((r-i)/1e3)),Da=k?.runId&&k?.gateRef?`${k.runId}
${k.gateRef}`:null;p.default.useEffect(()=>{if(!r)return;let ae=setInterval(()=>o(Date.now()),250);return()=>clearInterval(ae)},[r]),p.default.useEffect(()=>{at.current.gateKey!==Da&&(at.current={gateKey:Da,credentialRef:null,inFlight:!1})},[Da]),p.default.useEffect(()=>{if(!U2(k))return;let ae=Date.now(),ee=N=>{y5(N,k,ae)&&(pe(E=>U2(E)?null:E),U(!0))},Te=null;typeof window.BroadcastChannel=="function"&&(Te=new window.BroadcastChannel(c5),Te.onmessage=N=>ee(N.data));let Be=N=>{N.key===gh&&ee(yh(N.newValue))};window.addEventListener("storage",Be),ee(yh(window.localStorage?.getItem?.(gh)));let lt=window.setInterval(()=>{ee(yh(window.localStorage?.getItem?.(gh)))},500);return()=>{window.clearInterval(lt),Te&&Te.close(),window.removeEventListener("storage",Be)}},[k]);let Cn=T2({threadId:e,setMessages:_,setIsProcessing:U,setPendingGate:pe,setActiveRun:f,activeRunRef:d,locallyResolvedGatesRef:pt,toolActivityStateRef:bt,onRunSettled:(ae,{success:ee})=>{Ye.current=!1,ee&&y([]),S(void 0,{preserveClientOnly:!0,finalReplyTimestampByRun:ae&&ee?{[ae]:new Date().toISOString()}:null})}}),{status:Ot}=O2({threadId:e,onEvent:Cn,enabled:!!e}),Za=p.default.useCallback(async(ae,ee={})=>{let{threadId:Te,attachments:Be=[]}=ee,lt=Be.map(c$),N=Be.map(d$);if(k||Z.current)throw f5();if(Ye.current)return null;if(Be.length===0){let oe=await b5(ae);if(oe)return h(oe),{channel_connect_action:oe}}h(null);let E=Te||e;if(!E){let oe=await fc();if(At.invalidateQueries({queryKey:["threads"]}),E=oe?.thread?.thread_id,!E)throw new Error("createThread returned no thread_id")}let L=E,P={id:`pending-${n.current++}`,role:"user",content:ae,attachments:N,timestamp:new Date().toISOString(),isOptimistic:!0},F={id:P.id,role:"user",content:ae,attachments:N,timestamp:P.timestamp,isOptimistic:!0};A2(a.current,L,P);let B=P.id,G=!e||E===e,ie=oe=>{G&&_(oe)},he=oe=>{E!==e&&C(E,oe)},xt=oe=>{G&&oe()};Ye.current=!0,ie(oe=>[...oe,F]),he(oe=>[...oe,F]),xt(()=>{U(!0),Z.current||pe(null)});try{let oe=await qx({threadId:E,content:ae,attachments:lt});p5(E)&&At.invalidateQueries({queryKey:["threads"]}),oe?.run_id&&G&&f({runId:oe.run_id,threadId:oe.thread_id||E,status:oe.status||null,source:"local"});let Lt=M2(a.current,L,B,oe?.accepted_message_ref)||vh(oe?.accepted_message_ref);if(Lt){let da=$t=>$t.map(wt=>wt.id===B?{...wt,timelineMessageId:Lt}:wt);ie(da),he(da)}if(oe?.outcome==="deferred_busy"){let da=$t=>$t.map(wt=>wt.id===B?{...wt,isOptimistic:!1,status:"queued"}:wt);ie(da),he(da),Ye.current=!1}else if(oe?.outcome==="rejected_busy"){let da=$t=>$t.map(wt=>wt.id===B?{...wt,isOptimistic:!1,status:"error"}:wt);if(ie(da),he(da),oe?.notice){let $t=(fr=G)=>{let dl={id:`system-rejected-${n.current++}`,role:"system",content:oe.notice,timestamp:new Date().toISOString(),isOptimistic:!1},ml=fl=>[...fl,dl];fr&&_(ml),(!fr||E!==e)&&C(E,ml)};if(!t.current||t.current===E){let fr=P2(E,Z.current);fr?me({gateKey:fr,content:oe.notice}):$t()}else $t(!1)}xt(()=>U(!1))}return Ye.current=!1,oe}catch(oe){oe.status===429&&s(Date.now()+$5(oe));let Lt=da=>da.map($t=>$t.id===B?{...$t,isOptimistic:!1,status:"error",error:oe.message}:$t);throw ie(Lt),he(Lt),xt(()=>U(!1)),Ye.current=!1,oe}finally{D2(a.current,L,B)}},[e,k,_,C,U,pe,f]),ua=p.default.useCallback(async(ae,ee={})=>{if(!k)return;let{runId:Te,gateRef:Be}=k;if(!Te||!Be)throw new Error("resolveGate requires a pending gate with run_id and gate_ref");let lt=await Mp({threadId:e,runId:Te,gateRef:Be,resolution:ae,always:ee.always,credentialRef:ee.credentialRef}),N=v5(lt);if(pt.current.set(`${Te}
${Be}`,{resolution:ae,outcome:N}),x5(ae)&&N==="resumed"&&S2(_,k,bt),pe(null),N==="resumed"){U(!0),f({runId:lt?.run_id||Te,threadId:lt?.thread_id||e,status:lt?.status||"queued"});return}U(!1),f(null)},[k,e,_,f]),Wa=p.default.useCallback(async ae=>{if(!k)throw new Error("auth gate is no longer pending");let{runId:ee,gateRef:Te,provider:Be}=k;if(!ee||!Te||!Be)throw new Error("auth gate is missing required credential metadata");let lt=k.accountLabel||`${Be} credential`,N=`${ee}
${Te}`;if(at.current.gateKey!==N&&(at.current={gateKey:N,credentialRef:null,inFlight:!1}),at.current.inFlight)throw new Error("auth token submission already in progress");at.current.inFlight=!0;try{let E=at.current.credentialRef,L=null;if(!E){if(L=await L2(P=>Vx({provider:Be,accountLabel:lt,token:ae,threadId:e,runId:ee,gateRef:Te,signal:P})),E=L?.credential_ref,!E)throw new Error("manual token submit returned no credential_ref");at.current.credentialRef=E}if(!h5(L))try{await L2(P=>Mp({threadId:e,runId:ee,gateRef:Te,resolution:"credential_provided",credentialRef:E,signal:P}))}catch(P){throw m5(P)}at.current={gateKey:null,credentialRef:null,inFlight:!1},pe(null),U(!0)}catch(E){throw at.current.gateKey===N&&(at.current.inFlight=!1),E}},[k,e]),en=p.default.useCallback(async ae=>{let ee=u?.runId;!ee||!e||(pe(null),U(!1),f(null),Ye.current=!1,await Qx({threadId:e,runId:ee,reason:ae}))},[u,e]),ht=p.default.useCallback(()=>{g&&v&&S(v)},[g,v,S]),ca=p.default.useCallback(async(ae,ee,Te)=>{let Be="approved",lt=!1;ee==="deny"?Be="denied":ee==="cancel"?Be="cancelled":ee==="always"&&(Be="approved",lt=!0),await ua(Be,{always:lt})},[ua]),Vt=p.default.useCallback(()=>{},[]);return{messages:w,isProcessing:T,pendingGate:k,busyGateNotice:re,channelConnectAction:m,activeRun:u,sseStatus:Ot,historyLoading:x,historyLoadError:$,hasMore:g,cooldownSeconds:Rn,send:Za,resolveGate:ua,submitAuthToken:Wa,cancelRun:en,loadMore:ht,dismissChannelConnectAction:()=>h(null),suggestions:[],setSuggestions:Vt,retryMessage:Vt,approve:ca,recoverHistory:Vt,recoveryNotice:null}}function x5(e){return e==="denied"||e==="cancelled"}function $5(e){let t=e.headers?.get?.("Retry-After"),a=Number(t);return Number.isFinite(a)&&a>0?a*1e3:2e3}function F2({gatewayStatus:e,activeThread:t}){let a=t?.turn_count||0,n=e?.total_connections,r=e?.engine_v2_enabled===!1?"Engine v1":"Engine v2";return{mode:"Auto-review",runtime:"Work locally",workspace:"ironclaw",model:e?.llm_model,backend:e?.llm_backend,threadLabel:t?.title||"New thread",turnCountLabel:`${a} ${a===1?"turn":"turns"}`,engineLabel:r,connectionLabel:typeof n=="number"?`${n} live ${n===1?"connection":"connections"}`:null}}function w5(e){return{id:String(e?.id??`${e?.timestamp}:${e?.target}:${e?.message}`),timestamp:e?.timestamp||"",level:String(e?.level||"info").toLowerCase(),target:e?.target||"",message:e?.message||"",threadId:e?.thread_id||null,runId:e?.run_id||null,turnId:e?.turn_id||null,toolCallId:e?.tool_call_id||null,toolName:e?.tool_name||null,source:e?.source||null}}function Hc({threadId:e,runId:t,turnId:a,toolCallId:n,toolName:r,source:s}={},{absolute:i=!1}={}){let o=new URLSearchParams;e&&o.set("thread_id",e),t&&o.set("run_id",t),a&&o.set("turn_id",a),n&&o.set("tool_call_id",n),r&&o.set("tool_name",r),s&&o.set("source",s);let u=o.toString(),c=`/logs${u?`?${u}`:""}`;return i?`/v2${c}`:c}function B2(e){let t=e?.logs&&typeof e.logs=="object"?e.logs:e||{},a=Array.isArray(t.entries)?t.entries:[];return{source:t.source||"",entries:a.map(w5),nextCursor:t.next_cursor||null,tailSupported:!!t.tail_supported,followSupported:!!t.follow_supported}}var S5=1500;function z2({threads:e,activeThreadId:t,onSelectThread:a,isCreatingThread:n,composerDraft:r="",composerResetKey:s="",gatewayStatus:i}){let o=R(),{messages:u,isProcessing:c,pendingGate:d,busyGateNotice:f,channelConnectAction:m,suggestions:h,sseStatus:b,historyLoading:y,historyLoadError:w,hasMore:g,cooldownSeconds:v,recoveryNotice:x,activeRun:$,send:S,cancelRun:C,retryMessage:_,approve:T,recoverHistory:M,loadMore:O,setSuggestions:U,submitAuthToken:k,dismissChannelConnectAction:z}=j2(t),Z=p.default.useMemo(()=>e.find(ht=>ht.id===t)||null,[e,t]),re=p.default.useMemo(()=>F2({gatewayStatus:i,activeThread:Z}),[i,Z]),me=u.length>0||c||!!d||!!m,pe=!y&&!me&&!w,Ee=d?"Resolve the approval request before sending another message.":"",je=!!d||v>0,bt=p.default.useRef(je);bt.current=je;let pt=Ee||(v>0?`Retry in ${v}s`:void 0),at=t||Zs,Ye=t||Zs,Rn=!!(t&&$?.runId&&$.threadId===t&&c&&!d),Da=p.default.useMemo(()=>N5(d,u),[d,u]),Cn=t&&$?.runId&&$.threadId===t?Hc({threadId:t,runId:$.runId},{absolute:!0}):null,Ot=p.default.useCallback(async(ht,{images:ca=[],attachments:Vt=[]}={})=>{if(d)throw new Error(Ee);if(bt.current)return null;let ae=await S(ht,{images:ca,attachments:Vt,threadId:t}),ee=ae?.thread_id||t;return!t&&ee&&a&&a(ee,{replace:!0}),ae},[t,Ee,je,a,d,S]),Za=p.default.useCallback(async ht=>{je||(U([]),await Ot(ht))},[je,Ot,U]),ua=p.default.useCallback(()=>C("user_requested"),[C]);p.default.useEffect(()=>{if(!t)return;if(d){Sc(t,Sn.NEEDS_ATTENTION);return}if(c){Sc(t,Sn.RUNNING);return}let ht=setTimeout(()=>Tw(t),S5);return()=>clearTimeout(ht)},[t,d,c]);let[Wa,en]=p.default.useState(!1);return p.default.useEffect(()=>{let ht=ca=>{if(ca.key==="Escape"){en(!1);return}if(ca.key!=="?")return;let Vt=ca.target,ae=Vt?.tagName;ae==="INPUT"||ae==="TEXTAREA"||Vt?.isContentEditable||(ca.preventDefault(),en(ee=>!ee))};return window.addEventListener("keydown",ht),()=>window.removeEventListener("keydown",ht)},[]),l`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${M1} status=${b} />

        ${c&&!d&&Cn&&l`
          <div className="flex justify-end border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-1.5">
            <${$n}
              to=${Cn}
              className="inline-flex h-8 items-center gap-1.5 rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
              title=${o("nav.logs")}
            >
              <${D} name="list" className="h-3.5 w-3.5" />
              ${o("nav.logs")}
            <//>
          </div>
        `}

        ${w&&l`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${w}
          </div>
        `}

        ${pe&&l`
          <${O1}
            onSuggestion=${Za}
            onSend=${Ot}
            disabled=${!1}
            sendDisabled=${je}
            initialText=${r}
            resetKey=${s}
            draftKey=${at}
            autoFocusKey=${Ye}
            context=${re}
            statusText=${pt}
            canCancel=${Rn}
            onCancel=${ua}
          />
        `}
        ${!pe&&l`
          <${u2}
            messages=${u}
            isLoading=${y}
            hasMore=${g}
            onLoadMore=${O}
            onRetryMessage=${_}
            threadId=${t}
            pending=${c}
          >
            ${x&&l`
              <${c2}
                notice=${x}
                onRecover=${M}
              />
            `}
            ${c&&!d&&l`<${m2} />`}
            ${m&&l`
              <${T1}
                connectAction=${m}
                onDismiss=${z}
              />
            `}
            ${d&&(d.kind==="auth_required"?d.challengeKind==="oauth_url"?l`
                  <${R1}
                    gate=${d}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:d.challengeKind==="manual_token"?l`
                  <${C1}
                    gate=${d}
                    onSubmit=${k}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:l`
                  <${k1}
                    gate=${d}
                    onCancel=${()=>T(d.requestId,"cancel",d.kind)}
                  />
                `:l`
              <${_1}
                gate=${Da}
                onApprove=${()=>T(d.requestId,"approve",d.kind)}
                onDeny=${()=>T(d.requestId,"deny",d.kind)}
                onAlways=${()=>T(d.requestId,"always",d.kind)}
              />
            `)}
            ${f&&l`
              <div
                data-testid="busy-gate-notice"
                role="status"
                className="mx-auto mt-3 max-w-lg rounded-lg border border-copper/25 bg-copper/10 px-4 py-3 text-center text-sm leading-6 text-copper"
              >
                ${f.content}
              </div>
            `}
          <//>

          <${d2}
            suggestions=${h}
            onSelect=${Za}
            disabled=${je}
          />

          <${Lc}
            onSend=${Ot}
            disabled=${!1}
            sendDisabled=${je}
            initialText=${r}
            resetKey=${s}
            draftKey=${at}
            autoFocusKey=${Ye}
            context=${re}
            statusText=${pt}
            canCancel=${Rn}
            onCancel=${ua}
          />
        `}
      </div>
      <${L1}
        open=${Wa}
        onClose=${()=>en(!1)}
      />
    </div>
  `}function N5(e,t){if(!e||e.kind!=="gate")return e;let a=e.invocationId?_5(t,e.invocationId):null,n=R5(a)?a:k5(t,e.runId)||a,r=di(n?.toolParameters)||di(n?.toolDetail)||null;if(!r)return e;let s=Array.isArray(e.approvalDetails)?e.approvalDetails:[];return s.some(i=>C5(i?.label))?e.parameters?e:{...e,parameters:r}:{...e,approvalDetails:[...s,{label:"Arguments",value:r}],parameters:e.parameters||r}}function _5(e,t){for(let a of e||[]){if(a?.role==="tool_activity"&&a.invocationId===t)return a;let n=(a?.toolCalls||[]).find(r=>r?.invocationId===t);if(n)return n}return null}function k5(e,t){if(!t)return null;for(let a=(e||[]).length-1;a>=0;a-=1){let n=e[a];if(n?.turnRunId!==t)continue;if(n?.role==="tool_activity"&&di(n.toolParameters))return n;let r=n?.toolCalls||[];for(let s=r.length-1;s>=0;s-=1){let i=r[s];if(i?.turnRunId===t&&di(i.toolParameters))return i}}return null}function R5(e){return!!(di(e?.toolParameters)||di(e?.toolDetail))}function di(e){return typeof e=="string"&&e.trim()?e.trim():null}function C5(e){let t=typeof e=="string"?e.trim().toLowerCase():"";return t==="arguments"||t==="parameters"}function bh(){let{threadsState:e,gatewayStatus:t}=wa(),{threadId:a}=it(),n=fe(),r=Pe(),s=r.state?.composerDraft||"";p.default.useEffect(()=>{a&&a!==e.activeThreadId?e.setActiveThreadId(a):a||e.setActiveThreadId(null)},[a]);let i=p.default.useCallback((o,u={})=>{if(!o){e.setActiveThreadId(null),n("/chat",u);return}e.setActiveThreadId(o),n(`/chat/${o}`,u)},[e,n]);return l`
    <${z2}
      threads=${e.threads}
      activeThreadId=${e.activeThreadId}
      onSelectThread=${i}
      isCreatingThread=${e.isCreating}
      composerDraft=${s}
      composerResetKey=${r.key}
      gatewayStatus=${t}
    />
  `}function q2(e,t){return{name:e?.name||"",id:e?.id||"",adapter:e?.adapter||"open_ai_completions",baseUrl:e?ei(e,t):"",model:e?$c(e,t):""}}function I2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u}){let[c,d]=p.default.useState(()=>q2(e,a)),[f,m]=p.default.useState(""),[h,b]=p.default.useState([]),[y,w]=p.default.useState(null),[g,v]=p.default.useState(""),x=p.default.useRef(!!e);p.default.useEffect(()=>{n&&(d(q2(e,a)),m(""),b([]),w(null),v(""),x.current=!!e)},[n,e,a]);let $=e?.builtin===!0,S=e&&!e.builtin,C=p.default.useCallback((U,k)=>{d(z=>{let Z={...z,[U]:k};return U==="name"&&!x.current&&(Z.id=iw(k)),Z})},[]),_=p.default.useCallback(()=>!$&&(!c.name.trim()||!c.id.trim())?u("llm.fieldsRequired"):!$&&!ow(c.id.trim())?u("llm.invalidId"):!S&&!$&&t.includes(c.id.trim())?u("llm.idTaken",{id:c.id.trim()}):"",[t,c.id,c.name,$,S,u]),T=p.default.useCallback(async()=>{let U=_();if(U){w({tone:"error",text:U});return}v("save");try{await s({form:c,apiKey:f,provider:e}),r()}catch(k){w({tone:"error",text:k.message})}finally{v("")}},[f,c,r,s,e,_]),M=p.default.useCallback(async()=>{if(!c.model.trim()){w({tone:"error",text:u("llm.modelRequired")});return}v("test");try{let U=await i(Vp(e,c,f,a));w({tone:U.ok?"success":"error",text:U.message})}catch(U){w({tone:"error",text:U.message})}finally{v("")}},[f,a,c,i,e,u]),O=p.default.useCallback(async()=>{if(($?e?.base_url_required===!0:!0)&&!c.baseUrl.trim()){w({tone:"error",text:u("llm.baseUrlRequired")});return}v("models");try{let k=await o(Vp(e,c,f,a));if(!k.ok||!Array.isArray(k.models)||!k.models.length)w({tone:"error",text:k.message||u("llm.modelsFetchFailed")});else{b(k.models);let z=lw(c.model,k.models);z!==null&&C("model",z),w({tone:"success",text:u("llm.modelsFetched",{count:k.models.length})})}}catch(k){w({tone:"error",text:k.message})}finally{v("")}},[f,a,c,$,o,e,u,C]);return{form:c,apiKey:f,models:h,message:y,busy:g,isBuiltin:$,isEditing:S,setApiKey:m,update:C,submit:T,runTest:M,fetchModels:O,markIdEdited:()=>{x.current=!0}}}function Qc({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o}){let u=R(),c=I2({provider:e,allProviderIds:t,builtinOverrides:a,open:n,onClose:r,onSave:s,onTest:i,onListModels:o,t:u});if(!n)return null;let{form:d,apiKey:f,models:m,message:h,busy:b,isBuiltin:y,isEditing:w}=c,g=y?u("llm.configureProvider",{name:e.name||e.id}):u(w?"llm.editProvider":"llm.newProvider");return l`
    <${oi} open=${n} onClose=${r} title=${g} size="lg">
      <${li} className="space-y-4">
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
                disabled=${w}
                onChange=${v=>{c.markIdEdited(),c.update("id",v.target.value)}}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${u("llm.adapter")}
            <${ih} value=${d.adapter} onChange=${v=>c.update("adapter",v.target.value)}>
              ${Qp.map(v=>l`<option key=${v.value} value=${v.value}>${v.label}</option>`)}
            <//>
          </label>
        `}

        ${y&&l`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${Xo(e.adapter)}
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
            <${A} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${b!==""} onClick=${c.fetchModels}>
              ${u(b==="models"?"llm.fetchingModels":"llm.fetchModels")}
            <//>
          </div>
        </label>

        ${m.length>0&&l`
          <${ih} value=${d.model} onChange=${v=>c.update("model",v.target.value)}>
            ${m.map(v=>l`<option key=${v} value=${v}>${v}</option>`)}
          <//>
        `}

        ${h&&l`
          <div className=${h.tone==="error"?"text-sm text-red-200":"text-sm text-mint"} role="status">
            ${h.text}
          </div>
        `}
      <//>
      <${ui}>
        <${A} type="button" variant="secondary" disabled=${b!==""} onClick=${c.runTest}>
          ${u(b==="test"?"llm.testing":"llm.testConnection")}
        <//>
        <${A} type="button" variant="ghost" disabled=${b!==""} onClick=${r}>${u("common.cancel")}<//>
        <${A} type="button" disabled=${b!==""} onClick=${c.submit}>
          ${u(b==="save"?"common.saving":"common.save")}
        <//>
      <//>
    <//>
  `}function Vc({login:e}){let t=R(),{nearaiBusy:a,nearaiError:n,codexBusy:r,codexError:s,codexCode:i}=e;return l`
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
  `}function E5(e,t){if(!t)return!0;let a=t.toLowerCase();return[e.id,e.name,e.adapter,e.base_url,e.default_model].filter(Boolean).some(n=>String(n).toLowerCase().includes(a))}function Gc({settings:e,gatewayStatus:t,searchQuery:a,t:n}){let r=ti({settings:e,gatewayStatus:t}),[s,i]=p.default.useState(null),[o,u]=p.default.useState(!1),[c,d]=p.default.useState(null),f=p.default.useRef(null),m=p.default.useCallback((g,v)=>{f.current&&window.clearTimeout(f.current),d({tone:g,text:v}),f.current=window.setTimeout(()=>d(null),3500)},[]);p.default.useEffect(()=>()=>{f.current&&window.clearTimeout(f.current)},[]);let h=p.default.useCallback((g=null)=>{i(g),u(!0)},[]),b=p.default.useCallback(async g=>{try{await r.setActiveProvider(g),m("success",n("llm.providerActivated",{name:g.name||g.id}))}catch(v){v.message==="base_url"||v.message==="api_key"||v.message==="model"?(h(g),m("error",n(v.message==="base_url"?"llm.baseUrlRequired":v.message==="model"?"llm.modelRequired":"llm.configureToUse"))):m("error",v.message)}},[h,r,m,n]),y=p.default.useCallback(async({form:g,apiKey:v,provider:x})=>{if(x?.builtin){await r.saveBuiltinProvider({provider:x,form:g,apiKey:v}),m("success",n("llm.providerConfigured",{name:x.name||x.id}));return}let $=await r.saveCustomProvider({form:g,apiKey:v,editingProvider:x});m("success",n(x?"llm.providerUpdated":"llm.providerAdded",{name:$.name||$.id}))},[r,m,n]),w=p.default.useCallback(async g=>{if(window.confirm(n("llm.confirmDelete",{id:g.id})))try{await r.deleteCustomProvider(g),m("success",n("llm.providerDeleted"))}catch(v){m("error",v.message)}},[r,m,n]);return{providerState:r,dialogProvider:s,isDialogOpen:o,message:c,filteredProviders:r.providers.filter(g=>E5(g,a)),allProviderIds:r.providers.map(g=>g.id),openDialog:h,closeDialog:()=>u(!1),handleUse:b,handleSave:y,handleDelete:w}}var T5=3e5;function A5(){if(typeof window>"u"||!window.location)return!1;let e=window.location.hostname;return e==="localhost"||e==="0.0.0.0"||e==="::1"||/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(e)||e.endsWith(".localhost")}function D5(){return`nearai-wallet-login:${typeof window.crypto?.randomUUID=="function"?window.crypto.randomUUID():`${Date.now()}-${Math.random().toString(16).slice(2)}`}`}function M5(e,t){return new Promise(a=>{if(typeof window.BroadcastChannel!="function"){a(null);return}let n=new window.BroadcastChannel(t),r=u=>{let c=u.data;!c||c.type!=="nearai-wallet-login"||(o(),a(c.ok?c:null))},s=setInterval(()=>{e&&e.closed&&(o(),a(null))},500),i=setTimeout(()=>{o(),a(null)},T5);function o(){clearInterval(s),clearTimeout(i),n.removeEventListener("message",r),n.close()}n.addEventListener("message",r)})}var O5=3e5,L5=9e5,P5=2e3;async function K2(e,t,a){let n=Date.now()+t,r=2;for(;Date.now()<n;){if(await new Promise(i=>setTimeout(i,P5)),(await xc().catch(()=>null))?.active?.provider_id===e)return"active";if(a&&a.closed){if(r<=0)return"closed";r-=1}}return"timeout"}function Yc({onSuccess:e}={}){let t=R(),a=J(),[n,r]=p.default.useState(!1),[s,i]=p.default.useState(""),[o,u]=p.default.useState(!1),[c,d]=p.default.useState(""),[f,m]=p.default.useState(null),h=p.default.useCallback(()=>{i(""),d(""),m(null)},[]),b=p.default.useCallback(async()=>{await a.invalidateQueries({queryKey:["llm-providers"]}),e&&e()},[a,e]),y=p.default.useCallback(async v=>{if(h(),A5()){i(t("onboarding.nearaiLocalSso"));return}let x=window.open("about:blank","_blank");if(!x){i(t("onboarding.nearaiFailed"));return}try{x.opener=null}catch{}r(!0);try{let{auth_url:$}=await j$({provider:v,origin:window.location.origin});x.location.href=$;let S=await K2("nearai",O5,x);if(S==="active"){await b();return}x.close(),i(t(S==="closed"?"onboarding.nearaiFailed":"onboarding.nearaiTimeout"))}catch{x.close(),i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,h,t]),w=p.default.useCallback(async()=>{h(),r(!0);try{let v=D5(),x=window.open(`/v2/wallet/connect?channel=${encodeURIComponent(v)}`,"_blank","width=460,height=640");if(!x){i(t("onboarding.nearaiFailed"));return}x.opener=null;let $=await M5(x,v);if(!$){i(t("onboarding.nearaiFailed"));return}await F$({account_id:$.accountId,public_key:$.publicKey,signature:$.signature,message:$.message,recipient:$.recipient,nonce:$.nonce}),await b()}catch{i(t("onboarding.nearaiFailed"))}finally{r(!1)}},[b,h,t]),g=p.default.useCallback(async()=>{h();let v=window.open("about:blank","_blank");if(v)try{v.opener=null}catch{}u(!0);try{let{user_code:x,verification_uri:$}=await B$();m({userCode:x,verificationUri:$}),v&&(v.location.href=$);let S=await K2("openai_codex",L5,v);if(S==="active"){await b();return}v&&v.close(),d(t(S==="closed"?"onboarding.codexFailed":"onboarding.codexTimeout"))}catch{v&&v.close(),d(t("onboarding.codexFailed"))}finally{u(!1)}},[b,h,t]);return{nearaiBusy:n,nearaiError:s,codexBusy:o,codexError:c,codexCode:f,startNearai:y,startNearaiWallet:w,startCodex:g}}var H2="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",U5="M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z",j5="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z",F5="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z",B5={nearai:{color:"#00ec97",path:U5},openai_codex:{color:"#10a37f",path:H2},openai:{color:"#10a37f",path:H2},anthropic:{color:"#d97757",path:j5},ollama:{color:null,path:F5}};function Q2({id:e,name:t}){let a=B5[e],n="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-xl";if(!a){let s=(t||e||"?").trim().charAt(0).toUpperCase();return l`
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
  `}var z5=[{id:"nearai",auth:"nearai",nameKey:"onboarding.providerNearai",descKey:"onboarding.providerNearaiDesc"},{id:"openai_codex",auth:"codex",nameKey:"onboarding.providerCodex",descKey:"onboarding.providerCodexDesc"},{id:"openai",auth:"key",nameKey:"onboarding.providerOpenai",descKey:"onboarding.providerOpenaiDesc"},{id:"anthropic",auth:"key",nameKey:"onboarding.providerAnthropic",descKey:"onboarding.providerAnthropicDesc"},{id:"ollama",auth:"key",nameKey:"onboarding.providerOllama",descKey:"onboarding.providerOllamaDesc"}];function q5({provider:e,isBusy:t,login:a,t:n,onSetUp:r}){let[s,i]=p.default.useState(!1),o=p.default.useRef(null),u=t||a.nearaiBusy;p.default.useEffect(()=>{if(!s)return;let d=m=>{o.current&&!o.current.contains(m.target)&&i(!1)},f=m=>{m.key==="Escape"&&i(!1)};return document.addEventListener("mousedown",d),document.addEventListener("keydown",f),()=>{document.removeEventListener("mousedown",d),document.removeEventListener("keydown",f)}},[s]);let c=[{id:"api-key",label:n("llm.addApiKey"),disabled:t,run:()=>r(e)},{id:"near-wallet",label:n("onboarding.nearWallet"),disabled:a.nearaiBusy,run:a.startNearaiWallet},{id:"github",label:"GitHub",disabled:a.nearaiBusy,run:()=>a.startNearai("github")},{id:"google",label:"Google",disabled:a.nearaiBusy,run:()=>a.startNearai("google")}];return l`
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
  `}function I5({entry:e,provider:t,configured:a,isBusy:n,login:r,t:s,onUse:i,onSetUp:o}){let u=s(e.nameKey),c;return e.auth==="nearai"?c=l`<${q5} provider=${t} isBusy=${n} login=${r} t=${s} onSetUp=${o} />`:e.auth==="codex"?c=l`
      <${A} type="button" variant="secondary" size="sm" disabled=${r.codexBusy} onClick=${r.startCodex}>
        ${s("onboarding.signIn")}
      <//>
    `:a?c=l`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>i(t)}>
      ${s("llm.use")}
    <//>`:c=l`<${A} type="button" variant="primary" size="sm" disabled=${n} onClick=${()=>o(t)}>
      ${s("onboarding.setUp")}
    <//>`,l`
    <${te} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
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
  `}function V2(){let{isAdmin:e=!1,isChecking:t=!1}=wa();return t?null:e?l`<${K5} />`:l`<${ot} to="/chat" replace />`}function K5(){let e=R(),t=fe(),a=J(),{gatewayStatus:n}=wa(),r=Gc({settings:{},gatewayStatus:n,searchQuery:"",t:e}),s=r.providerState,i=z5.map(f=>({entry:f,provider:s.providers.find(m=>m.id===f.id)})).filter(f=>f.provider),o=p.default.useCallback(()=>t("/chat"),[t]),u=Yc({onSuccess:o}),c=p.default.useCallback(async f=>{let m=f.active_model||f.default_model||"";await Jo({provider_id:f.id,model:m}),await a.invalidateQueries({queryKey:["llm-providers"]}),t("/chat")},[t,a]),d=p.default.useCallback(async({form:f,apiKey:m,provider:h})=>{await r.handleSave({form:f,apiKey:m,provider:h});let b=h?.id||f.id.trim(),y=f.model?.trim()||h?.default_model||"";await Jo({provider_id:b,model:y}),await a.invalidateQueries({queryKey:["llm-providers"]}),r.closeDialog(),t("/chat")},[r,t,a]);return s.isLoading?l`
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
              <${I5}
                key=${f.id}
                entry=${f}
                provider=${m}
                configured=${zr(m,s.builtinOverrides)}
                isBusy=${s.isBusy}
                login=${u}
                t=${e}
                onUse=${c}
                onSetUp=${r.openDialog}
              />
            `)}
        </div>

        <${Vc} login=${u} />

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

      <${Qc}
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
  `}function I({children:e,className:t="",...a}){return l`<${te} className=${t} ...${a}>${e}<//>`}function et({label:e,value:t,tone:a="muted",badgeLabel:n,detail:r,showDivider:s=!0,className:i="",valueClassName:o="text-[1.75rem] md:text-[2rem]"}){return l`
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
  `;return n?l`<${te} padding="lg">${r}<//>`:l`<div className="py-8">${r}</div>`}var Y2={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function Ja({result:e,onDismiss:t}){return e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",Y2[e.type]||Y2.info].join(" ")}>
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">Dismiss</button>
    </div>
  `:null}var J2="",H5={workspace:"home"};function Jc(e){return H5[e]||e}function rl(e){return[...e||[]].sort((t,a)=>t.is_dir!==a.is_dir?t.is_dir?-1:1:t.name.localeCompare(a.name,void 0,{sensitivity:"base"}))}function mi(e){return e?e.split("/").filter(Boolean):[]}function Xc(e){return e?`/workspace/${mi(e).map(encodeURIComponent).join("/")}`:"/workspace"}function xh(e){let t=mi(e);return t.pop(),t.join("/")}function X2(e){return/\.mdx?$/i.test(e||"")}function Zc({path:e,onNavigate:t}){let a=R(),n=mi(e),r="";return l`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${()=>t("/workspace")}
        className="text-signal hover:underline"
      >
        ${a("workspace.breadcrumbRoot")}
      </button>
      ${n.map((s,i)=>{r=r?`${r}/${s}`:s;let o=r,u=i===0?Jc(s):s;return l`
          <span key=${o} className="text-iron-400">/</span>
          <button
            key=${`${o}-button`}
            type="button"
            onClick=${()=>t(Xc(o))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${u}
          </button>
        `})}
    </div>
  `}function Q5(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function Z2({path:e,entries:t,isLoading:a,filter:n,onOpen:r,onNavigate:s}){let i=R();if(a)return l`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;let o=(t||[]).filter(m=>!Q5(m.path)),u=String(n||"").trim().toLowerCase(),c=u?o.filter(m=>m.name.toLowerCase().includes(u)):o,d=rl(c),f;return o.length?d.length?f=l`
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
    <${I} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Zc} path=${e} onNavigate=${s} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">${f}</div>
    <//>
  `}var Wc="/api/webchat/v2/fs",V5=1024*1024,G5=8*1024*1024;function W2(e){let t=String(e||"").split("/").filter(Boolean);return{mount:t.shift()||"",path:t.join("/")}}function Y5(e,t){return t?`${e}/${t}`:e}function J5(e){let t=String(e||"").toLowerCase();return t.startsWith("text/")||t==="application/json"||t==="application/javascript"||t==="application/xml"||t.endsWith("+json")||t.endsWith("+xml")}function X5(e){return String(e||"").toLowerCase().startsWith("image/")}function Z5(e){let t=String(e||"").toLowerCase();return t.startsWith("audio/")||t.startsWith("video/")||t.startsWith("font/")||t==="application/pdf"||t==="application/zip"||t==="application/gzip"}function W5(e){if(e.subarray(0,Math.min(e.length,8192)).indexOf(0)!==-1)return!0;try{return new TextDecoder("utf-8",{fatal:!0}).decode(e),!1}catch{return!0}}function eD(e,t){let a=new URL(`${Wc}/content`,window.location.origin);return a.searchParams.set("mount",e),a.searchParams.set("path",t),a.pathname+a.search}async function tD(){return(await H(`${Wc}/mounts`))?.mounts||[]}async function fi(e=""){if(!e)return{entries:(await tD()).map(o=>({name:Jc(o.mount),path:o.mount,is_dir:!0}))};let{mount:t,path:a}=W2(e),n=new URL(`${Wc}/list`,window.location.origin);return n.searchParams.set("mount",t),a&&n.searchParams.set("path",a),{entries:((await H(n.pathname+n.search))?.entries||[]).map(i=>({name:i.name,path:Y5(t,i.path),is_dir:i.kind==="directory"}))}}async function eS(e){let{mount:t,path:a}=W2(e);if(!t||!a)return{kind:"directory",path:e};let n=new URL(`${Wc}/stat`,window.location.origin);n.searchParams.set("mount",t),n.searchParams.set("path",a);let s=(await H(n.pathname+n.search))?.stat||{},i=s.mime_type||"application/octet-stream",o=Number(s.size_bytes||0),u=eD(t,a),c={path:e,mime:i,size_bytes:o,download_path:u};if(s.kind&&s.kind!=="file")return{...c,kind:"directory"};if(X5(i)){if(o>G5)return{...c,kind:"binary"};let h=await hc(u);return{...c,kind:"image",image_data_url:h}}if(Z5(i)||o>V5)return{...c,kind:"binary"};let d=await Ca(u),f=new Uint8Array(await d.arrayBuffer());if(!J5(i)&&W5(f))return{...c,kind:"binary"};let m=new TextDecoder("utf-8").decode(f);return{...c,kind:"text",content:m}}function tS(e=""){return String(e).split("/").some(t=>t.startsWith("."))}function aD(e,t,a){let n=String(t||"").trim().toLowerCase(),r=(e||[]).filter(s=>!tS(s.path)).filter(s=>!n||s.name.toLowerCase().includes(n)?!0:s.is_dir&&a.has(s.path));return rl(r)}function aS({entry:e,depth:t,selectedPath:a,expandedPaths:n,filter:r,onToggleDirectory:s,onSelectFile:i}){let o=R(),u=n.has(e.path),c=K({queryKey:["workspace-list",e.path],queryFn:()=>fi(e.path),enabled:e.is_dir&&u});if(e.is_dir){let d=aD(c.data?.entries,r,n);return l`
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
                  <${aS}
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
  `}function nS({entries:e,selectedPath:t,expandedPaths:a,filter:n,onToggleDirectory:r,onSelectFile:s,isLoading:i}){let o=R();if(i)return l`<div className="space-y-2 p-3">${[1,2,3,4].map(c=>l`<div key=${c} className="v2-skeleton h-8 rounded-md" />`)}</div>`;let u=rl(e.filter(c=>!tS(c.path)));return u.length?l`
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
  `}function sS(e){return mi(e).pop()||"download"}function nD({path:e,file:t}){let a=R();return t.kind==="image"?l`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${t.image_data_url}
          alt=${sS(e)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `:t.kind==="text"?l`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${X2(e)?l`<${ia} content=${t.content} className="max-w-4xl text-base leading-7" />`:l`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${t.content}</pre>`}
      </div>
    `:l`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${a("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `}function iS({path:e,file:t,isLoading:a,onNavigate:n}){let r=R(),[s,i]=p.default.useState(!1),o=p.default.useCallback(async()=>{if(t?.download_path){i(!0);try{let c=await Ca(t.download_path);Pc(c,sS(e))}catch{}finally{i(!1)}}},[t,e]);if(a)return l`
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
        <${Zc} path=${e} onNavigate=${n} />
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

      <${nD} path=${e} file=${t} />

      ${xh(e)&&l`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${r("workspace.parent",{path:xh(e)})}
        </div>
      `}
    <//>
  `}function oS(e){let t=R(),a=J(),[n,r]=p.default.useState(new Set),[s,i]=p.default.useState(""),[o,u]=p.default.useState(null),c=K({queryKey:["workspace-list",""],queryFn:()=>fi("")}),d=K({queryKey:["workspace-file",e],queryFn:()=>eS(e),enabled:!!e}),f=e===""||d.data?.kind==="directory",m=K({queryKey:["workspace-list",e],queryFn:()=>fi(e),enabled:f});p.default.useEffect(()=>{u(null)},[e]);let h=p.default.useCallback(y=>a.fetchQuery({queryKey:["workspace-list",y],queryFn:()=>fi(y)}),[a]),b=p.default.useCallback(async y=>{let w=new Set(n);if(w.has(y)){w.delete(y),r(w);return}w.add(y),r(w);try{await h(y)}catch(g){u({type:"error",message:g.message||t("workspace.unableOpenDirectory")})}},[n,h,t]);return{rootEntries:c.data?.entries||[],file:d.data||null,selectionIsDirectory:f,currentEntries:m.data?.entries||[],expandedPaths:n,filter:s,setFilter:i,result:o,clearResult:()=>u(null),isLoadingTree:c.isLoading,isLoadingFile:d.isLoading,isLoadingListing:m.isLoading,isFetching:c.isFetching||d.isFetching||m.isFetching,error:c.error||d.error||m.error||null,loadDirectory:h,toggleDirectory:b,refresh:()=>{a.invalidateQueries({queryKey:["workspace-list"]}),a.invalidateQueries({queryKey:["workspace-file",e]})}}}function $h(){let e=R(),t=fe(),n=it()["*"]||J2,r=oS(n),s=p.default.useCallback(i=>{t(Xc(i))},[t]);return l`
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
          <${Ja}
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
  `}function lS(e){if(!e)return null;let t=e.metadata&&typeof e.metadata=="object"&&!Array.isArray(e.metadata)?e.metadata:{};return{id:e.project_id,name:e.name,description:e.description,goals:Array.isArray(t.goals)?t.goals:[],icon:e.icon||null,color:e.color||null,state:e.state,role:e.role,metadata:t,created_at:e.created_at,updated_at:e.updated_at,health:e.state==="archived"?"muted":"green"}}async function uS(){let t=((await Px({limit:200}))?.projects||[]).map(lS);return{attention:[],projects:t}}async function cS(e){if(!e)return null;let t=await Ux({projectId:e});return lS(t?.project)}function dS(e){return Promise.resolve({missions:[],todo:!0})}function mS(e){return Promise.resolve({threads:[],todo:!0})}function fS(e){return Promise.resolve({widgets:[],todo:!0})}function pS(e){return Promise.resolve(null)}function hS(e){return Promise.resolve(null)}function vS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function gS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function yS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function bS(){let e=J(),t=K({queryKey:["projects-overview"],queryFn:uS,refetchInterval:5e3}),a=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]})},[e]);return{overview:t.data||{attention:[],projects:[]},isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null,invalidate:a}}function xS(e){let t=J(),a=!!e,n=K({queryKey:["project-detail",e],queryFn:()=>cS(e),enabled:a,refetchInterval:a?7e3:!1}),r=K({queryKey:["project-missions",e],queryFn:()=>dS(e),enabled:a,refetchInterval:a?5e3:!1}),s=K({queryKey:["project-threads",e],queryFn:()=>mS(e),enabled:a,refetchInterval:a?4e3:!1}),i=K({queryKey:["project-widgets",e],queryFn:()=>fS(e),enabled:a,refetchInterval:a?15e3:!1}),o=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["projects-overview"]}),t.invalidateQueries({queryKey:["project-detail",e]}),t.invalidateQueries({queryKey:["project-missions",e]}),t.invalidateQueries({queryKey:["project-threads",e]}),t.invalidateQueries({queryKey:["project-widgets",e]})},[e,t]);return{project:n.data||null,missions:r.data?.missions||[],threads:s.data?.threads||[],widgets:i.data||[],isLoading:a&&(n.isLoading||r.isLoading||s.isLoading),isRefreshing:n.isFetching||r.isFetching||s.isFetching||i.isFetching,error:n.error||r.error||s.error||i.error||null,invalidate:o}}function $S({projectId:e,missionId:t,threadId:a}){let n=J(),[r,s]=p.default.useState(null),i=K({queryKey:["project-mission-detail",t],queryFn:()=>pS(t),enabled:!!t,refetchInterval:t?5e3:!1}),o=K({queryKey:["project-thread-detail",a],queryFn:()=>hS(a),enabled:!!a,refetchInterval:a?4e3:!1}),u=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["projects-overview"]}),n.invalidateQueries({queryKey:["project-detail",e]}),n.invalidateQueries({queryKey:["project-missions",e]}),n.invalidateQueries({queryKey:["project-threads",e]}),t&&n.invalidateQueries({queryKey:["project-mission-detail",t]}),a&&n.invalidateQueries({queryKey:["project-thread-detail",a]})},[t,e,n,a]),c=Q({mutationFn:({targetMissionId:m})=>vS(m),onSuccess:m=>{s({type:"success",message:m?.thread_id?"Mission fired and a new run is live.":"Mission fire request accepted."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to fire mission"})}}),d=Q({mutationFn:({targetMissionId:m})=>gS(m),onSuccess:()=>{s({type:"success",message:"Mission paused."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to pause mission"})}}),f=Q({mutationFn:({targetMissionId:m})=>yS(m),onSuccess:()=>{s({type:"success",message:"Mission resumed."}),u()},onError:m=>{s({type:"error",message:m.message||"Unable to resume mission"})}});return{mission:i.data?.mission||null,thread:o.data?.thread||null,inspectorType:a?"thread":t?"mission":null,isLoading:i.isLoading||o.isLoading,isRefreshing:i.isFetching||o.isFetching,error:i.error||o.error||null,actionResult:r,clearActionResult:()=>s(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending}}function ed(e){if(!e)return"No recent activity";let t=new Date(e),a=Date.now()-t.getTime(),n=Math.abs(a),r=a<0;if(n<6e4)return r?"in under a minute":"just now";if(n<36e5){let i=Math.floor(n/6e4);return r?`in ${i}m`:`${i}m ago`}if(n<864e5){let i=Math.floor(n/36e5);return r?`in ${i}h`:`${i}h ago`}let s=Math.floor(n/864e5);return r?`in ${s}d`:`${s}d ago`}function td(e){return new Intl.NumberFormat(void 0,{style:"currency",currency:"USD",maximumFractionDigits:e>=100?0:2}).format(Number(e||0))}function wS(e){return e==="green"?"success":e==="yellow"?"warning":e==="red"?"danger":"muted"}function SS(e){return e==="Running"?"signal":e==="Done"||e==="Completed"?"success":e==="Failed"?"danger":"warning"}function rD(e){let t=String(e||"").trim();if(!t)return null;let a=t.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);if(a)return{missionName:a[1].trim(),missionBrief:a[2].trim()};let n=t.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);return n?{missionName:n[1].trim(),missionBrief:n[2].trim()}:null}function NS(e){let t=rD(e?.goal);return t?{title:t.missionName,subtitle:"Mission run",brief:t.missionBrief}:{title:e?.title||e?.goal||`Thread ${(e?.id||"").slice(0,8)}`,subtitle:e?.thread_type?String(e.thread_type).replace(/_/g," "):"Thread",brief:e?.title&&e?.goal&&e.title!==e.goal?e.goal:""}}function _S(e){let t=e?.projects||[],a=t.reduce((o,u)=>o+Number(u.cost_today_usd||0),0),n=t.reduce((o,u)=>o+Number(u.active_missions||0),0),r=t.reduce((o,u)=>o+Number(u.threads_today||0),0),s=t.reduce((o,u)=>o+Number(u.pending_gates||0),0),i=t.reduce((o,u)=>o+Number(u.failures_24h||0),0);return{totalProjects:t.length,activeMissions:n,threadsToday:r,totalSpend:a,pendingGates:s,failures24h:i,attentionCount:e?.attention?.length||0}}function sl(e,t){return`${e} ${t}${e===1?"":"s"}`}var sD={projects:"muted",attention:"warning",spend:"success"};function kS({overview:e}){let t=_S(e),a=[{key:"projects",label:"Projects",value:t.totalProjects,detail:`${t.threadsToday} threads active today`},{key:"attention",label:"Attention queue",value:t.attentionCount,detail:`${t.failures24h} failures in the last 24h`},{key:"spend",label:"Spend today",value:td(t.totalSpend),detail:`${t.totalProjects?"Across every project":"Waiting for activity"}`}];return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${a.map(n=>l`
          <div key=${n.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${n.label}</div>
              <${q} tone=${sD[n.key]} label=${n.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${n.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${n.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `}function iD(e){return e?.type==="failure"?"danger":"warning"}function oD(e){return e?.type==="failure"?"failure":"gate"}function RS({items:e,onOpenItem:t}){return e?.length?l`
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
              <${q} tone=${iD(a)} label=${oD(a)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${a.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `:null}function lD({project:e,onOpen:t,t:a}){return l`
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
            ${a("projects.card.threadsToday",{count:sl(e.threads_today||0,"thread")})}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${a("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${sl(e.pending_gates||0,"gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${a("projects.card.failures24h",{count:sl(e.failures_24h||0,"failure")})}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${a("projects.card.spendToday",{value:td(e.cost_today_usd||0)})}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${ed(e.last_activity)}</div>
        </div>
        <${A}
          variant="secondary"
          onClick=${n=>{n.stopPropagation(),t(e.id)}}
        >${a("projects.openWorkspace")}<//>
      </div>
    </article>
  `}function uD({project:e,onOpen:t,t:a}){return l`
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
            ${sl(e.threads_today||0,"thread")} today
          </div>
          <${A}
            variant="secondary"
            onClick=${n=>{n.stopPropagation(),t(e.id)}}
          >${a("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `}function CS({projects:e,totalProjects:t,search:a,onSearchChange:n,onOpenProject:r,onCreateProject:s,isPreparingChat:i}){let o=R(),u=e.find(d=>d.name==="default"),c=e.filter(d=>d.name!=="default");return!e.length&&t>0?l`
      <${xe}
        title=${o("projects.empty.noMatchTitle")}
        description=${o("projects.empty.noMatchDesc")}
      />
    `:e.length?l`
    <div className="space-y-5">
      ${u&&l`<${uD} project=${u} onOpen=${r} t=${o} />`}

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

      ${c.length?l`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${c.map(d=>l`<${lD} key=${d.id} project=${d} onOpen=${r} t=${o} />`)}
          </div>`:l`
            <${xe}
              title=${o("projects.scoped.onlyGeneralTitle")}
              description=${o("projects.scoped.onlyGeneralDesc")}
            >
              <${A} onClick=${s}>${o(i?"projects.preparingChat":"projects.startProject")}<//>
            <//>
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
                    <span>${ed(i.updated_at||i.created_at)}</span>
                  </div>
                </button>
              `}):l`
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                No project threads yet. When an automation runs or scoped chat work happens inside this project, activity will appear here.
              </div>
            `}
      </div>
    <//>
  `}var cD="/workspace";function dD(e){let t=a=>a.kind==="directory"?0:1;return[...e].sort((a,n)=>t(a)-t(n)||a.name.localeCompare(n.name,void 0,{sensitivity:"base"}))}function mD(e){return e?String(e).replace(/^\/workspace\/?/,"").split("/").filter(Boolean):[]}function TS({threadId:e}){let t=R(),[a,n]=p.default.useState(void 0),[r,s]=p.default.useState(null),i=K({queryKey:["project-files",e||"",a||""],queryFn:()=>Ex({threadId:e,path:a}),enabled:!!e}),o=p.default.useMemo(()=>dD(i.data?.entries||[]),[i.data]),u=p.default.useCallback(async f=>{if(f.kind==="directory"){s(null),n(f.path);return}try{s(null);let m=await Ca(pc({threadId:e,path:f.path})),h=URL.createObjectURL(m),b=document.createElement("a");b.href=h,b.download=f.name,document.body.appendChild(b),b.click(),b.remove(),URL.revokeObjectURL(h)}catch(m){s(m?.message||"Unable to download file")}},[e]),c=mD(a),d=l`
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
        ${c.map((f,m)=>{let h=`${cD}/${c.slice(0,m+1).join("/")}`;return l`
            <span key=${h} className="text-iron-500">/</span>
            <button
              key=${`${h}-button`}
              type="button"
              onClick=${()=>n(h)}
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
      <${I} className="p-4 sm:p-5">
        ${d}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet \u2014 they appear once a thread has run in this project."}
        </div>
      <//>
    `}function fD(e){return[...e||[]].sort((a,n)=>new Date(n.updated_at||n.created_at)-new Date(a.updated_at||a.created_at))[0]?.id||null}function AS({project:e,threads:t,selectedThreadId:a,onSelectThread:n,onNewConversation:r,isStartingConversation:s}){let i=fD(t);return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight text-white">${e.name}</h2>
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
  `}function il(){let e=R(),t=fe(),{threadsState:a}=wa(),{projectId:n=null,threadId:r=null}=it(),[s,i]=p.default.useState(""),[o,u]=p.default.useState(null),c=bS(),d=xS(n),f=$S({projectId:n,threadId:r}),m=p.default.useMemo(()=>{let _=s.trim().toLowerCase();return _?c.overview.projects.filter(T=>[T.name,T.description,...T.goals||[]].some(M=>String(M||"").toLowerCase().includes(_))):c.overview.projects},[c.overview.projects,s]),h=p.default.useMemo(()=>c.overview.projects.find(_=>_.id===n)||null,[c.overview.projects,n]),b=p.default.useCallback(()=>{c.invalidate(),d.invalidate()},[c,d]),y=p.default.useCallback(_=>{t(`/projects/${_}`)},[t]),w=p.default.useCallback(_=>{if(_.thread_id){t(`/projects/${_.project_id}/threads/${_.thread_id}`);return}t(`/projects/${_.project_id}`)},[t]),g=p.default.useCallback(async()=>{let _=null;u(null);try{_=await a.createThread()}catch(T){u({type:"error",message:T.message||e("projects.chatAutoFail")})}t("/chat",{state:{composerDraft:e("projects.creationDraft"),threadId:_}})},[t,a]),v=p.default.useCallback(_=>{t(`/projects/${n}/threads/${_}`)},[t,n]),x=p.default.useCallback(async()=>{u(null);try{let _=await a.createThread(n);t("/chat",{state:{threadId:_}}),d.invalidate()}catch(_){u({type:"error",message:_.message||e("projects.chatAutoFail")})}},[t,a,n,d,e]),$=p.default.useCallback(()=>{t(`/projects/${n}`)},[t,n]),S=l`
    ${n&&l`<${A} variant="ghost" onClick=${()=>t("/projects")}>${e("projects.allProjects")}<//>`}
  `,C=null;return n?d.isLoading?C=l`
        <div className="space-y-4">
          ${[1,2,3].map(_=>l`<div key=${_} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `:d.error||!d.project&&!h?C=l`
        <${xe}
          title=${e("projects.unavailable")}
          description=${d.error?.message||e("projects.unavailableDesc")}
        >
          <${A} variant="secondary" onClick=${()=>t("/projects")}>${e("projects.returnToProjects")}<//>
        <//>
      `:C=l`
        <${AS}
          project=${d.project||h}
          threads=${d.threads}
          selectedThreadId=${r}
          onSelectThread=${v}
          onNewConversation=${x}
          isStartingConversation=${a.isCreating}
        />
      `:C=c.isLoading?l`
          <div className="space-y-4">
            ${[1,2,3].map(_=>l`<div key=${_} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `:l`
          <${CS}
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
          <${Ja} result=${o} onDismiss=${()=>u(null)} />
          <${Ja} result=${f.actionResult} onDismiss=${f.clearActionResult} />
          ${!n&&l`
            <${kS} overview=${c.overview} />
            <${RS} items=${c.overview.attention} onOpenItem=${w} />
          `}
          ${C}
        </div>
      </div>
    </div>
  `}function ol(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not scheduled"}function ll(e){return e==="Active"?"signal":e==="Paused"?"warning":e==="Completed"?"success":e==="Failed"?"danger":"muted"}function DS(e=[]){return e.reduce((t,a)=>(t.total+=1,a.status==="Active"?t.active+=1:a.status==="Paused"?t.paused+=1:a.status==="Completed"?t.completed+=1:a.status==="Failed"&&(t.failed+=1),t.threads+=Number(a.thread_count||a.threads?.length||0),t),{total:0,active:0,paused:0,completed:0,failed:0,threads:0})}function MS(e=[]){let t={Active:0,Paused:1,Failed:2,Completed:3};return[...e].sort((a,n)=>{let r=(t[a.status]??4)-(t[n.status]??4);return r!==0?r:new Date(n.updated_at||0).getTime()-new Date(a.updated_at||0).getTime()})}function ad({label:e,value:t}){return l`
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t}</div>
    </div>
  `}function pD({mission:e,isBusy:t,onFire:a,onPause:n,onResume:r}){let s=R();return e.status==="Active"?l`
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
          <${q} tone=${ll(e.status)} label=${e.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${ad} label=${c("missions.meta.cadence")} value=${e.cadence_description||e.cadence_type||c("missions.meta.manual")} />
          <${ad} label=${c("missions.meta.threadsToday")} value=${`${e.threads_today||0} / ${e.max_threads_per_day||c("missions.meta.unlimited")}`} />
          <${ad} label=${c("missions.meta.nextFire")} value=${ol(e.next_fire_at)} />
          <${ad} label=${c("missions.meta.updated")} value=${ol(e.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <${pD}
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
          <${ia} content=${e.goal||c("missions.noGoal")} />
        </div>
      <//>

      ${e.current_focus&&l`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${ia} content=${e.current_focus} />
          </div>
        <//>
      `}

      ${e.success_criteria&&l`
        <${I} className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${c("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <${ia} content=${e.success_criteria} />
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
                  <${q} tone=${ll(d.state==="Running"?"Active":d.state==="Failed"?"Failed":"Completed")} label=${d.state} />
                </div>
              </button>
            `)}
          </div>
        <//>
      `:null}
    </div>
  `}function hD(e){return[{value:"all",label:e("missions.filter.allStatuses")},{value:"Active",label:e("missions.status.active")},{value:"Paused",label:e("missions.status.paused")},{value:"Failed",label:e("missions.status.failed")},{value:"Completed",label:e("missions.status.completed")}]}function LS({value:e,onChange:t,children:a,label:n}){return l`
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
  `}function vD({mission:e,selectedMissionId:t,onSelectMission:a,onOpenProject:n}){let r=R(),s=t===e.id;return l`
    <div
      className=${["w-full rounded-xl border p-4 text-left",s?"border-signal/35 bg-signal/10":"border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80"].join(" ")}
    >
      <button type="button" onClick=${()=>a(e.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${e.name}</div>
              <${q} tone=${ll(e.status)} label=${e.status} />
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
          ${r("missions.updated",{value:ol(e.updated_at)})}
        </span>
        <${A}
          variant="ghost"
          onClick=${i=>{i.stopPropagation(),n(e.project.id)}}
        >
          ${e.project.name}
        <//>
      </div>
    </div>
  `}function wh({missions:e,totalMissions:t,selectedMissionId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,projectFilter:o,onProjectFilterChange:u,projectOptions:c,onSelectMission:d,onOpenProject:f}){let m=R(),h=hD(m);return l`
    <${I} className="p-4 sm:p-5">
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
        <${LS} value=${s} onChange=${i} label=${m("missions.filter.status")}>
          ${h.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}<//>`)}
        <//>
        <${LS} value=${o} onChange=${u} label=${m("missions.filter.project")}>
          <option value="all">${m("missions.filter.allProjects")}</option>
          ${c.map(b=>l`<option key=${b.id} value=${b.id}>${b.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${e.length?e.map(b=>l`
              <${vD}
                key=${b.id}
                mission=${b}
                selectedMissionId=${a}
                onSelectMission=${d}
                onOpenProject=${f}
              />
            `):l`
              <${xe}
                title=${m("missions.emptyTitle")}
                description=${m("missions.emptyDesc")}
                boxed=${!1}
              />
            `}
      </div>
    <//>
  `}function gD(e){return[{key:"total",label:e("missions.summary.totalMissions"),tone:"muted"},{key:"active",label:e("missions.summary.active"),tone:"signal"},{key:"paused",label:e("missions.summary.paused"),tone:"warning"},{key:"threads",label:e("missions.summary.spawnedThreads"),tone:"success"}]}function PS({summary:e}){let t=R(),a=gD(t);return l`
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
  `}function US(){return Promise.resolve({projects:[],todo:!0})}function jS({projectId:e}={}){return Promise.resolve({missions:[],todo:!0})}function FS(e){return Promise.resolve(null)}function BS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function zS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function qS(e){return Promise.resolve({success:!1,message:"TODO: requires v2 missions endpoint"})}function IS(e){let t=K({queryKey:["mission-detail",e],queryFn:()=>FS(e),enabled:!!e,refetchInterval:e?5e3:!1});return{mission:t.data?.mission||null,isLoading:t.isLoading,isRefreshing:t.isFetching,error:t.error||null}}function yD(e,t){return{...e,project:{id:t.id,name:t.name,health:t.health}}}function KS(){let e=J(),[t,a]=p.default.useState(null),n=K({queryKey:["projects-overview"],queryFn:US,refetchInterval:7e3}),r=n.data?.projects||[],s=Ad({queries:r.map(m=>({queryKey:["missions","project",m.id],queryFn:()=>jS({projectId:m.id}),refetchInterval:5e3,select:h=>h?.missions||[]}))}),i=s.flatMap((m,h)=>{let b=r[h];return(m.data||[]).map(y=>yD(y,b))}),o=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["projects-overview"]}),e.invalidateQueries({queryKey:["missions"]}),e.invalidateQueries({queryKey:["mission-detail"]})},[e]),u=(m,h)=>({mutationFn:({missionId:b})=>m(b),onSuccess:()=>{a({type:"success",message:h}),o()},onError:b=>{a({type:"error",message:b.message||"Unable to update mission"})}}),c=Q(u(BS,"Mission fired and a run was queued.")),d=Q(u(zS,"Mission paused.")),f=Q(u(qS,"Mission resumed."));return{projects:r,missions:i,summary:DS(i),isLoading:n.isLoading||s.some(m=>m.isLoading),isRefreshing:n.isFetching||s.some(m=>m.isFetching),error:n.error||s.find(m=>m.error)?.error||null,actionResult:t,clearActionResult:()=>a(null),fireMission:c.mutateAsync,pauseMission:d.mutateAsync,resumeMission:f.mutateAsync,isBusy:c.isPending||d.isPending||f.isPending,invalidate:o}}function Sh(){let e=R(),t=fe(),{missionId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,u]=p.default.useState("all"),c=KS(),d=IS(a),f=p.default.useMemo(()=>{let g=n.trim().toLowerCase();return MS(c.missions).filter(v=>{let x=!g||[v.name,v.goal,v.project?.name].some(C=>String(C||"").toLowerCase().includes(g)),$=s==="all"||v.status===s,S=o==="all"||v.project?.id===o;return x&&$&&S})},[c.missions,o,n,s]),m=p.default.useMemo(()=>c.missions.find(g=>g.id===a)||null,[a,c.missions]),h=d.mission?{...m,...d.mission,project:m?.project||null}:m,b=p.default.useCallback(g=>{g.project_id&&t(`/projects/${g.project_id}/threads/${g.id}`)},[t]),y=p.default.useCallback(async(g,v)=>{try{await g({missionId:v})}catch{}},[]),w=a?l`
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <${wh}
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
          <${OS}
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
      `:l`
        <${wh}
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

          <${Ja}
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
              `:w}
        </div>
      </div>
    </div>
  `}var HS=[{id:"overview",label:"Overview"},{id:"activity",label:"Activity"},{id:"files",label:"Files"}],bD=new Set(["pending","in_progress"]),QS=new Set(["failed","interrupted","stuck","cancelled"]);function ir(e){return e?String(e).replace(/_/g," "):"unknown"}function pi(e){return e?e==="completed"||e==="accepted"||e==="submitted"?"success":e==="in_progress"?"signal":e==="pending"?"warning":QS.has(e)?"danger":"muted":"muted"}function xD(e){return bD.has(e)}function nd(e){return xD(e?.state)}function VS(e){return e?.can_restart?e.job_kind==="sandbox"?e.state==="failed"||e.state==="interrupted":QS.has(e.state):!1}function Ir(e,t=8){return e?String(e).slice(0,t):"unknown"}function oa(e,t={}){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...t}):"Not available"}function GS(e){if(e==null)return"Not available";if(e<60)return`${e}s`;let t=Math.floor(e/60),a=e%60;return t<60?`${t}m ${a}s`:`${Math.floor(t/60)}h ${t%60}m`}function Nh(e){return[e?.job_kind?`${e.job_kind} job`:null,e?.job_mode?e.job_mode.replace(/^acp:/,"acp "):null,e?.started_at?`started ${oa(e.started_at)}`:null].filter(Boolean).join(" / ")}var $D=[{value:"all",label:"All events"},{value:"message",label:"Messages"},{value:"tool_use",label:"Tool calls"},{value:"tool_result",label:"Tool results"},{value:"status",label:"Status"},{value:"result",label:"Final results"}];function YS(e){if(typeof e=="string")return e;try{return JSON.stringify(e,null,2)}catch{return String(e)}}function wD({event:e}){let{event_type:t,data:a}=e;return t==="tool_use"||t==="tool_result"?l`
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
  `}function JS({job:e,events:t,onSendPrompt:a,isSendingPrompt:n}){let r=R(),[s,i]=p.default.useState("all"),[o,u]=p.default.useState(""),[c,d]=p.default.useState(!0),f=p.default.useRef(null),m=p.default.useMemo(()=>s==="all"?t:t.filter(b=>b.event_type===s),[t,s]);p.default.useEffect(()=>{c&&f.current&&(f.current.scrollTop=f.current.scrollHeight)},[c,m.length]);let h=p.default.useCallback(async(b=!1)=>{let y=o.trim();if(!(!y&&!b))try{await a({content:y||"(done)",done:b}),u("")}catch{}},[o,a]);return l`
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
            ${$D.map(b=>l`<option key=${b.value} value=${b.value}>${b.label}</option>`)}
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
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${oa(b.created_at)}</div>
                <${wD} event=${b} />
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
            onInput=${b=>u(b.target.value)}
            onKeyDown=${b=>{b.key==="Enter"&&!b.shiftKey&&(b.preventDefault(),h(!1))}}
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
              <${q} tone=${pi(e.state)} label=${ir(e.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
              <span>${Ir(e.id)}</span>
              <span>created ${oa(e.created_at)}</span>
              ${Nh(e)&&l`<span>${Nh(e)}</span>`}
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
            ${nd(e)&&l`
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
          ${s?l`<div className="space-y-2 px-2">${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-8 rounded-md" />`)}</div>`:t.length?l`
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

        ${r&&!i?l`<div className="mt-5 rounded-md border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${r}</div>`:i?l`<div className="mt-5 space-y-3">${[1,2,3,4,5].map(f=>l`<div key=${f} className="v2-skeleton h-4 rounded" />`)}</div>`:n?l`<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-white/10 bg-iron-950/90 p-4 font-mono text-xs leading-6 text-iron-100">${n.content}</pre>`:l`
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
    `}function hi({label:e,value:t}){return l`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${e}</div>
      <div className="mt-2 text-sm leading-6 text-white">${t||"Not available"}</div>
    </div>
  `}function eN({job:e}){let t=(e.transitions||[]).map(a=>({title:`${ir(a.from)} -> ${ir(a.to)}`,description:[oa(a.timestamp),a.reason].filter(Boolean).join(" / ")}));return l`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${I} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${q} tone=${pi(e.state)} label=${ir(e.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${hi} label="Created" value=${oa(e.created_at)} />
          <${hi} label="Started" value=${oa(e.started_at)} />
          <${hi} label="Completed" value=${oa(e.completed_at)} />
          <${hi} label="Duration" value=${GS(e.elapsed_secs)} />
          <${hi} label="Kind" value=${e.job_kind?`${e.job_kind} job`:null} />
          <${hi} label="Mode" value=${e.job_mode||"Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${e.description?l`<${ia} content=${e.description} className="mt-4 text-sm leading-7 text-iron-200" />`:l`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
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
  `}function tN({jobs:e,totalJobs:t,selectedJobId:a,search:n,onSearchChange:r,stateFilter:s,onStateFilterChange:i,onSelectJob:o,onCancelJob:u,isBusy:c,isRefreshing:d}){let f=R(),m=[{value:"all",label:f("jobs.list.filter.all")},{value:"pending",label:f("jobs.list.filter.pending")},{value:"in_progress",label:f("jobs.list.filter.inProgress")},{value:"completed",label:f("jobs.list.filter.completed")},{value:"failed",label:f("jobs.list.filter.failed")},{value:"stuck",label:f("jobs.list.filter.stuck")}];if(!e.length){let h=!!n.trim()||s!=="all";return l`
      <${xe}
        title=${f(t&&h?"jobs.list.empty.noMatchTitle":"jobs.list.empty.noJobsTitle")}
        description=${f(t&&h?"jobs.list.empty.noMatchDesc":"jobs.list.empty.noJobsDesc")}
      />
    `}return l`
    <div className="space-y-5">
      <${I} className="p-4 sm:p-5">
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
            onInput=${h=>r(h.target.value)}
            placeholder=${f("jobs.list.searchPlaceholder")}
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${h=>i(h.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${m.map(h=>l`<option key=${h.value} value=${h.value}>${h.label}</option>`)}
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
                  <h3 className="truncate text-lg font-semibold text-iron-100">${h.title||f("jobs.list.untitled")}</h3>
                  <${q} tone=${pi(h.state)} label=${ir(h.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${Ir(h.id)}</span>
                  <span>${f("jobs.list.created",{value:oa(h.created_at)})}</span>
                  ${h.started_at&&l`<span>${f("jobs.list.started",{value:oa(h.started_at)})}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${nd(h)&&l`
                  <${A}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${c}
                    onClick=${()=>u(h.id)}
                  >
                    ${f("jobs.action.cancel")}
                  <//>
                `}
                <${A} variant="ghost" className="h-9 px-3 text-xs" onClick=${()=>o(h.id)}>${f("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `}var SD=[{key:"total",label:"Total jobs",tone:"muted",detail:"All tracked work across agent and sandbox execution."},{key:"pending",label:"Pending",tone:"warning",detail:"Queued work waiting for a worker or container slot."},{key:"in_progress",label:"In progress",tone:"signal",detail:"Actively running jobs and live bridges."},{key:"completed",label:"Completed",tone:"success",detail:"Finished without intervention."},{key:"failed",label:"Failed",tone:"danger",detail:"Runs that terminated with an error or interruption."},{key:"stuck",label:"Stuck",tone:"danger",detail:"Agent work needing recovery or operator attention."}];function aN({summary:e}){return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${SD.map(t=>l`
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
  `}function nN(){return Promise.resolve({jobs:[],pagination:null,todo:!0})}function rN(){return Promise.resolve({total:0,active:0,completed:0,failed:0,todo:!0})}function sN(e){return Promise.resolve(null)}function iN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function oN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function lN(e){return Promise.resolve({events:[],todo:!0})}function uN(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 jobs endpoint"})}function _h(e,t=""){return Promise.resolve({entries:[],todo:!0})}function cN(e,t){return Promise.resolve({content:"",todo:!0})}function dN(e){let t=J(),[a,n]=p.default.useState(null),r=K({queryKey:["job-detail",e],queryFn:()=>sN(e),enabled:!!e,refetchInterval:e?4e3:!1}),s=K({queryKey:["job-events",e],queryFn:()=>lN(e),enabled:!!e,refetchInterval:e?2500:!1}),i=Q({mutationFn:({content:o,done:u})=>uN(e,{content:o,done:u}),onSuccess:(o,{done:u})=>{n({type:"success",message:u?"Done signal sent to the job":"Follow-up sent to the job"}),t.invalidateQueries({queryKey:["job-detail",e]}),t.invalidateQueries({queryKey:["job-events",e]}),t.invalidateQueries({queryKey:["jobs"]}),t.invalidateQueries({queryKey:["jobs-summary"]})},onError:o=>{n({type:"error",message:o.message||"Unable to send follow-up"})}});return p.default.useEffect(()=>{n(null)},[e]),{job:r.data||null,events:s.data?.events||[],isLoading:r.isLoading,isRefreshing:r.isFetching||s.isFetching,error:r.error||s.error||null,sendPrompt:i.mutateAsync,isSendingPrompt:i.isPending,promptResult:a,clearPromptResult:()=>n(null)}}function mN(e=[]){return e.map(t=>({name:t.name,path:t.path,isDir:t.is_dir,children:t.is_dir?[]:null,loaded:!1,expanded:!1}))}function fN(e,t){for(let a of e){if(a.path===t)return a;if(a.children?.length){let n=fN(a.children,t);if(n)return n}}return null}function rd(e,t,a){return e.map(n=>n.path===t?a(n):n.children?.length?{...n,children:rd(n.children,t,a)}:n)}function pN(e){let[t,a]=p.default.useState([]),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,u]=p.default.useState(""),c=!!(e?.project_dir&&e?.id),d=K({queryKey:["job-files-root",e?.id],queryFn:()=>_h(e.id,""),enabled:c}),f=K({queryKey:["job-file",e?.id,n],queryFn:()=>cN(e.id,n),enabled:!!(c&&n)});p.default.useEffect(()=>{a([]),r(""),i(""),u("")},[e?.id]),p.default.useEffect(()=>{d.data?.entries?(a(mN(d.data.entries)),i("")):d.error&&i(d.error.message||"Unable to load project files")},[d.data,d.error]);let m=p.default.useCallback(async h=>{let b=fN(t,h);if(!(!b||!e?.id)){if(b.expanded){a(y=>rd(y,h,w=>({...w,expanded:!1})));return}if(b.loaded){a(y=>rd(y,h,w=>({...w,expanded:!0})));return}u(h);try{let y=await _h(e.id,h);a(w=>rd(w,h,g=>({...g,expanded:!0,loaded:!0,children:mN(y.entries)}))),i("")}catch(y){i(y.message||"Unable to open folder")}finally{u("")}}},[e?.id,t]);return{canBrowse:c,tree:t,selectedPath:n,selectPath:r,selectedFile:f.data||null,fileError:f.error?.message||"",isLoadingTree:d.isLoading,isLoadingFile:f.isLoading||f.isFetching,expandingPath:o,treeError:s,toggleDirectory:m}}function hN(){let e=J(),[t,a]=p.default.useState(null),n=K({queryKey:["jobs-summary"],queryFn:rN,refetchInterval:5e3}),r=K({queryKey:["jobs"],queryFn:nN,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["jobs"]}),e.invalidateQueries({queryKey:["jobs-summary"]})},[e]),i=Q({mutationFn:({jobId:u})=>iN(u),onSuccess:(u,{jobId:c})=>{a({type:"success",message:`Job ${Ir(c)} cancelled`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to cancel job"})}}),o=Q({mutationFn:({jobId:u})=>oN(u),onSuccess:u=>{a({type:"success",message:`Restart queued as ${Ir(u?.new_job_id)}`}),s()},onError:u=>{a({type:"error",message:u.message||"Unable to restart job"})}});return{summary:n.data||{total:0,pending:0,in_progress:0,completed:0,failed:0,stuck:0},jobs:r.data?.jobs||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),cancelJob:i.mutateAsync,restartJob:o.mutateAsync,isBusy:i.isPending||o.isPending,invalidate:s}}function vN({result:e,onDismiss:t}){let a=R();if(!e)return null;let n={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};return l`
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
  `}function kh(){let e=R(),t=fe(),{jobId:a=null}=it(),[n,r]=p.default.useState(""),[s,i]=p.default.useState("all"),[o,u]=p.default.useState(a?"activity":"overview"),c=hN(),d=dN(a),f=pN(d.job);p.default.useEffect(()=>{u(a?"activity":"overview")},[a]);let m=p.default.useMemo(()=>{let v=n.trim().toLowerCase();return c.jobs.filter(x=>{let $=!v||x.title.toLowerCase().includes(v)||x.id.toLowerCase().includes(v),S=s==="all"||x.state===s;return $&&S})},[c.jobs,n,s]),h=p.default.useCallback(v=>t(`/jobs/${v}`),[t]),b=p.default.useCallback(async v=>{try{await c.cancelJob({jobId:v})}catch{}},[c]),y=p.default.useCallback(async v=>{try{let x=await c.restartJob({jobId:v});x?.new_job_id&&t(`/jobs/${x.new_job_id}`)}catch{}},[c,t]),w=l`
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
        <${XS}
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
          <${tN}
            jobs=${m}
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
  `}function or(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"}):"Not scheduled"}function sd(e,t=!0){return!t||e==="disabled"?"muted":e==="active"?"signal":e==="running"?"warning":e==="failing"||e==="attention"?"danger":"muted"}function id(e){return e==="verified"?"success":e==="unverified"?"warning":"muted"}function gN(e=[]){return[...e].sort((t,a)=>t.enabled!==a.enabled?t.enabled?-1:1:new Date(a.next_fire_at||a.last_run_at||0).getTime()-new Date(t.next_fire_at||t.last_run_at||0).getTime())}function yN(e){return!e||typeof e!="object"?"No action details":e.type?e.type:e.Lightweight?"lightweight":e.FullJob?"full job":"configured"}function ND(e){return e==="ok"?"success":e==="running"?"warning":"danger"}function bN({runs:e}){return e?.length?l`
    <div className="space-y-3">
      ${e.map(t=>l`
          <div key=${t.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${q} tone=${ND(t.status)} label=${t.status} />
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
  `}function xN({title:e,value:t}){return l`
    <div>
      <h3 className="text-sm font-semibold text-iron-100">${e}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >${JSON.stringify(t||{},null,2)}</pre>
    </div>
  `}function $N({routine:e,isLoading:t,error:a,isBusy:n,onTriggerRoutine:r,onToggleRoutine:s,onDeleteRoutine:i}){let o=fe(),u=R();return t?l`
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
              tone=${sd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${id(e.verification_status)}
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
        <${lr} label="Action" value=${yN(e.action)} />
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
              tone=${sd(e.status,e.enabled)}
              label=${e.enabled?e.status:"disabled"}
            />
            <${q}
              tone=${id(e.verification_status)}
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
  `}var _D=[{value:"all",label:"All routines"},{value:"enabled",label:"Enabled"},{value:"disabled",label:"Disabled"},{value:"unverified",label:"Unverified"},{value:"failing",label:"Failing"}];function Rh({routines:e,totalRoutines:t,selectedRoutineId:a,search:n,onSearchChange:r,statusFilter:s,onStatusFilterChange:i,onSelectRoutine:o,onTriggerRoutine:u,onToggleRoutine:c,isBusy:d,isRefreshing:f}){let m=R();if(!e.length){let h=!!n.trim()||s!=="all";return l`
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
            onInput=${h=>r(h.target.value)}
            placeholder="Search routine name, trigger, or action"
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${s}
            onChange=${h=>i(h.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${_D.map(h=>l`<option key=${h.value} value=${h.value}>${h.label}<//>`)}
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
  `}var kD=[{key:"total",label:"Total routines",tone:"muted",detail:"All saved schedules and event handlers."},{key:"enabled",label:"Enabled",tone:"signal",detail:"Ready to run from schedule, event, or manual trigger."},{key:"disabled",label:"Disabled",tone:"muted",detail:"Paused until explicitly re-enabled."},{key:"unverified",label:"Unverified",tone:"warning",detail:"Needs a successful validation run."},{key:"failing",label:"Failing",tone:"danger",detail:"Recent run status needs operator attention."},{key:"runs_today",label:"Runs today",tone:"success",detail:"Routines with activity since local day start."}];function SN({summary:e}){return l`
    <${I} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${kD.map(t=>l`
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
  `}function NN(e){let[t,a]=p.default.useState(""),[n,r]=p.default.useState("all");return{filteredRoutines:p.default.useMemo(()=>{let i=t.trim().toLowerCase();return gN(e).filter(o=>{let u=[o.name,o.description,o.trigger_summary,o.trigger_type,o.action_type,o.status].join(" ").toLowerCase(),c=!i||u.includes(i),d=n==="all"||n==="enabled"&&o.enabled||n==="disabled"&&!o.enabled||n==="unverified"&&o.verification_status==="unverified"||n==="failing"&&o.status==="failing";return c&&d})},[e,t,n]),search:t,setSearch:a,statusFilter:n,setStatusFilter:r}}function _N(){return Promise.resolve({routines:[],todo:!0})}function kN(){return Promise.resolve({total:0,active:0,paused:0,todo:!0})}function RN(e){return Promise.resolve(null)}function od(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function ld(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function CN(e){return Promise.resolve({success:!1,message:"TODO: requires v2 routines endpoint"})}function EN(e){let t=J(),[a,n]=p.default.useState(null),r=K({queryKey:["routine-detail",e],queryFn:()=>RN(e),enabled:!!e,refetchInterval:e?5e3:!1}),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["routine-detail",e]}),t.invalidateQueries({queryKey:["routines"]}),t.invalidateQueries({queryKey:["routines-summary"]})},[t,e]),i=(c,d)=>({mutationFn:()=>c(e),onSuccess:()=>{n({type:"success",message:d}),s()},onError:f=>{n({type:"error",message:f.message||"Unable to update routine"})}}),o=Q(i(od,"Routine run queued.")),u=Q(i(ld,"Routine status updated."));return{routine:r.data||null,isLoading:r.isLoading,error:r.error||null,actionResult:a,clearActionResult:()=>n(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,isBusy:o.isPending||u.isPending}}function TN(){let e=J(),[t,a]=p.default.useState(null),n=K({queryKey:["routines-summary"],queryFn:kN,refetchInterval:5e3}),r=K({queryKey:["routines"],queryFn:_N,refetchInterval:5e3}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["routines"]}),e.invalidateQueries({queryKey:["routines-summary"]}),e.invalidateQueries({queryKey:["routine-detail"]})},[e]),i=(d,f)=>({mutationFn:({routineId:m})=>d(m),onSuccess:()=>{a({type:"success",message:f}),s()},onError:m=>{a({type:"error",message:m.message||"Unable to update routine"})}}),o=Q(i(od,"Routine run queued.")),u=Q(i(ld,"Routine status updated.")),c=Q(i(CN,"Routine deleted."));return{summary:n.data||{total:0,enabled:0,disabled:0,unverified:0,failing:0,runs_today:0},routines:r.data?.routines||[],isLoading:n.isLoading||r.isLoading,isRefreshing:n.isFetching||r.isFetching,error:n.error||r.error||null,actionResult:t,clearActionResult:()=>a(null),triggerRoutine:o.mutateAsync,toggleRoutine:u.mutateAsync,deleteRoutine:c.mutateAsync,isBusy:o.isPending||u.isPending||c.isPending,invalidate:s}}function Ch(){let e=fe(),{routineId:t=null}=it(),a=TN(),n=EN(t),r=NN(a.routines),s=p.default.useCallback(async(u,c)=>{try{await u({routineId:c})}catch{}},[]),i=p.default.useCallback(async(u,c)=>{if(window.confirm(`Delete routine "${c}"?`))try{await a.deleteRoutine({routineId:u}),e("/routines")}catch{}},[e,a]),o=t?l`
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <${Rh}
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
        <${Rh}
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

          <${Ja}
            result=${a.actionResult}
            onDismiss=${a.clearActionResult}
          />
          <${Ja}
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
  `}function RD(e){return e==="available"?"success":e==="unavailable"?"warning":"muted"}function CD(e,t){return e.split(/(\{[^}]+\})/).map((n,r)=>{let s=n.match(/^\{(.+)\}$/)?.[1];return s&&t[s]!=null?t[s]:n})}function AN({deliveryState:e}){let t=R(),a=e.currentTarget?.target_id||"",[n,r]=p.default.useState(a),[s,i]=p.default.useState(!1),o=p.default.useRef(null);p.default.useEffect(()=>{r(a)},[a]),p.default.useEffect(()=>()=>{o.current&&clearTimeout(o.current)},[]);let u=n!==a,c=e.isLoading||e.isSaving,d=u&&!c,f=!!a&&!c,m=e.finalReplyTargets.length>0,h=e.targets.some(M=>M?.capabilities?.final_replies&&M?.target?.status==="unavailable"),b=m||h,y=M=>(o.current&&clearTimeout(o.current),i(!1),M.then(()=>{o.current&&clearTimeout(o.current),i(!0),o.current=setTimeout(()=>i(!1),2200)}).catch(()=>{})),w=()=>{d&&y(e.saveFinalReplyTarget(n||null))},g=()=>{f&&(r(""),y(e.saveFinalReplyTarget(null)))},v=e.currentTarget?.display_name||t("automations.delivery.none"),x=e.currentStatus,$=x==="available"?"success":x==="unavailable"?"warning":"muted",S=t(x==="available"?"automations.delivery.pill.ready":x==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.notSet"),C=!!e.currentTarget,_=t(C?"automations.delivery.changeTarget":"automations.delivery.availableTargets"),T=CD(t("automations.delivery.footnote"),{command:l`<code
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
        ${C&&l`
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
              <${q} tone=${$} label=${S} />
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
            ${e.finalReplyTargets.map(M=>{let O=M?.target?.target_id??"",U=M?.target?.display_name||M?.target?.target_id||"",k=M?.target?.description||"",z=M?.target?.status??"available",Z=n===O;return l`
                <label
                  key=${O}
                  className=${V("flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer","transition-colors duration-100","bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)]","hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",Z&&"border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]")}
                >
                  <input
                    type="radio"
                    name="delivery-target"
                    value=${O}
                    checked=${Z}
                    disabled=${c}
                    onChange=${()=>r(O)}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                      ${U}
                    </div>
                    ${k&&l`<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                      ${k}
                    </div>`}
                  </div>
                  <${q}
                    tone=${RD(z)}
                    label=${t(z==="unavailable"?"automations.delivery.pill.unavailable":"automations.delivery.pill.ready")}
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
            onClick=${w}
          >
            <${D} name="check" className="h-3.5 w-3.5" />
            ${t("automations.delivery.save")}
          <//>
          <${A}
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
            ${T}
          </div>
        `}

      </div>
    <//>
  `}var ED=["schedule","once"],MN={active:{labelKey:"automations.state.active",tone:"signal"},scheduled:{labelKey:"automations.state.scheduled",tone:"signal"},paused:{labelKey:"automations.state.paused",tone:"warning"},disabled:{labelKey:"automations.state.disabled",tone:"warning"},inactive:{labelKey:"automations.state.inactive",tone:"warning"},completed:{labelKey:"automations.state.completed",tone:"success"},unknown:{labelKey:"automations.state.unknown",tone:"muted"}},ON={ok:{labelKey:"automations.lastStatus.done",tone:"success"},error:{labelKey:"automations.lastStatus.error",tone:"danger"},running:{labelKey:"automations.lastStatus.running",tone:"info"}},LN={ok:{labelKey:"automations.runStatus.ok",tone:"success"},error:{labelKey:"automations.runStatus.error",tone:"danger"},running:{labelKey:"automations.runStatus.running",tone:"info"},unknown:{labelKey:"automations.runStatus.unknown",tone:"muted"}};function la(e){return typeof e=="function"?e:t=>t}var Th=[{value:"all",labelKey:"automations.filter.all",predicate:null},{value:"active",labelKey:"automations.filter.active",predicate:kn},{value:"running",labelKey:"automations.filter.running",predicate:e=>e.has_running_run},{value:"failures",labelKey:"automations.filter.failures",predicate:e=>e.has_failed_runs},{value:"paused",labelKey:"automations.filter.paused",predicate:KD},{value:"completed",labelKey:"automations.filter.completed",predicate:HD}];function PN(e,t,a){return(Array.isArray(e?.automations)?e.automations:[]).filter(r=>ED.includes(r?.source?.type)).map(r=>FD(r,t,a)).sort(ID)}function UN(e,t){let a=Th.find(n=>n.value===t)?.predicate;return a?e.filter(a):e}function jN(e){let t=e.filter(i=>i.state!=="completed"),a=t.filter(i=>kn(i)).length,n=t.filter(i=>i.has_running_run).length,r=t.filter(i=>i.has_failed_runs).length,s=t.filter(i=>kn(i)&&Eh(i)!=null).sort((i,o)=>(i.next_run_timestamp??Number.MAX_SAFE_INTEGER)-(o.next_run_timestamp??Number.MAX_SAFE_INTEGER))[0];return{scheduled:t.length,active:a,running:n,failures:r,nextRun:s?.next_run_label||null}}function TD(e,t,a,n){let r=typeof a=="function"?a:g=>g;if(!e||typeof e!="string")return r("automations.schedule.custom");let s=YD(e);if(!s)return r("automations.schedule.custom");let{minute:i,hour:o,dayOfMonth:u,month:c,dayOfWeek:d,year:f}=s,m=t&&typeof t=="string"?t:null,h=m?` (${m})`:"",b=f==="*"&&u==="*"&&c==="*"&&d==="*";if(b&&o==="*"){if(i==="*")return r("automations.schedule.everyMinute");let g=JD(i);if(g===1)return r("automations.schedule.everyMinute");if(g)return r("automations.schedule.everyMinutes",{count:g});if(ur(i,0,59))return r("automations.schedule.hourlyAt",{minute:String(Number(i)).padStart(2,"0")})}let y=QD(o,i,n);if(!y)return r("automations.schedule.custom");if(b)return r("automations.schedule.everyDayAt",{time:y})+h;let w=XD(d);if(f==="*"&&u==="*"&&c==="*"&&w==="1-5")return r("automations.schedule.weekdaysAt",{time:y})+h;if(f==="*"&&u==="*"&&c==="*"&&ur(w,0,7)){let g=VD(Number(w)%7,n);return r("automations.schedule.weekdayAt",{weekday:g,time:y})+h}if(f==="*"&&ur(u,1,31)&&c==="*"&&d==="*")return r("automations.schedule.monthlyAt",{day:Number(u),time:y})+h;if(ur(u,1,31)&&ur(c,1,12)&&d==="*"&&(f==="*"||ur(f,1970,9999))){let g=GD(Number(c),Number(u),f==="*"?null:Number(f),n);return r("automations.schedule.dateAt",{date:g,time:y})+h}return r("automations.schedule.custom")}function Kr(e,t="Unknown",a,n){if(!e)return t;let r=new Date(e);if(Number.isNaN(r.getTime()))return t;let s=n&&typeof n=="string"?{timeZone:n}:{};try{return r.toLocaleString(a||[],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit",...s})}catch{return r.toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}}function FN(e,t){let a=MN[e]?.labelKey||"automations.state.unknown";return la(t)(a)}function BN(e){return MN[e]?.tone||"muted"}function AD(e,t){return kn(e)&&e?.has_running_run?la(t)("automations.status.running"):kn(e)&&e?.has_failed_runs?la(t)("automations.status.needsReview"):FN(e?.state,t)}function DD(e){return kn(e)&&e?.has_running_run?"info":kn(e)&&e?.has_failed_runs?"danger":BN(e?.state)}function MD(e,t){let a=ON[e]?.labelKey||"automations.lastStatus.none";return la(t)(a)}function OD(e){return ON[e]?.tone||"muted"}function LD(e,t){let a=LN[ud(e)]?.labelKey||"automations.runStatus.unknown";return la(t)(a)}function PD(e){return LN[ud(e)]?.tone||"muted"}function UD(e,t,a,n){if(!e)return la(a)("automations.schedule.custom");let r=Kr(e,null,n,t);if(!r)return la(a)("automations.schedule.custom");let s=t&&typeof t=="string"?` (${t})`:"";return la(a)("automations.schedule.onceAt",{datetime:r})+s}function jD(e,t,a){return e?.type==="once"?UD(e.at,e.timezone,t,a):e?.type==="schedule"?TD(e.cron,e.timezone||"UTC",t,a):la(t)("automations.schedule.custom")}function FD(e,t,a){let n=la(t),r=BD(e.recent_runs,t,a),s=r[0]||null,i=r.find(f=>f.status==="running")||null,o=r.find(f=>f.status==="ok"||f.status==="error")||null,u=o?.status||e.last_status,c=o?.completed_at||e.last_run_at||null,d={...e,recent_runs:r,has_running_run:r.some(f=>f.status==="running"),has_failed_runs:r.some(f=>f.status==="error")};return{...d,display_name:e.name||n("automations.untitled"),schedule_timezone:e.source?.timezone||"UTC",schedule_label:jD(e.source,t,a),state_label:FN(e.state,t),state_tone:BN(e.state),primary_status_label:AD(d,t),primary_status_tone:DD(d),next_run_timestamp:Ah(e.next_run_at),next_run_label:Kr(e.next_run_at,n("automations.date.notScheduled"),a),last_run_label:Kr(c,n("automations.date.noRuns"),a),last_status_label:MD(u,t),last_status_tone:OD(u),created_label:Kr(e.created_at,n("automations.date.unknown"),a),latest_run:s,current_run:i,success_rate_label:qD(r,t)}}function BD(e,t,a){let n=la(t);return Array.isArray(e)?e.map(r=>{let s=ud(r?.status),i=r?.fired_at||r?.fire_slot||r?.submitted_at||r?.completed_at||null,o=Ah(i);return{...r,status:s,status_label:LD(s,t),status_tone:PD(s),timestamp:o,timestamp_source:i,fired_label:Kr(i,n("automations.date.unscheduled"),a),submitted_label:Kr(r?.submitted_at,n("automations.date.notSubmitted"),a),completed_label:Kr(r?.completed_at,n("automations.date.notCompleted"),a),chat_path:r?.thread_id?`/chat/${encodeURIComponent(r.thread_id)}`:null}}).sort((r,s)=>(s.timestamp??0)-(r.timestamp??0)):[]}function ud(e){return e==="ok"||e==="error"||e==="running"?e:"unknown"}function zN(e){let t=Array.isArray(e)?e:[],a={total:t.length,ok:0,error:0,running:0,unknown:0};for(let n of t){let r=ud(n?.status);Object.prototype.hasOwnProperty.call(a,r)?a[r]+=1:a.unknown+=1}return a}function zD(e){let t=zN(e);return[{key:"ok",tone:"text-emerald-300",count:t.ok},{key:"error",tone:"text-red-300",count:t.error},{key:"running",tone:"text-sky-300",count:t.running},{key:"unknown",tone:"text-iron-400",count:t.unknown}].filter(a=>a.count>0)}function qN(e,t){let a=la(t),n=zN(e),r=zD(e).map(s=>({...s,text:a(`automations.runs.${s.key}`,{count:s.count})}));return{total:n.total,totalText:a("automations.runs.total",{count:n.total}),chips:r}}function qD(e,t){let a=la(t),n=e.filter(s=>s.status==="ok"||s.status==="error");if(!n.length)return a("automations.successRate.none");let r=n.filter(s=>s.status==="ok").length;return a("automations.successRate.visible",{percent:Math.round(r/n.length*100)})}function ID(e,t){let a=kn(e),n=kn(t);return a!==n?a?-1:1:(Eh(e)??Number.MAX_SAFE_INTEGER)-(Eh(t)??Number.MAX_SAFE_INTEGER)}function Ah(e){if(!e)return null;let t=new Date(e);return Number.isNaN(t.getTime())?null:t.getTime()}function kn(e){return e?.state==="active"||e?.state==="scheduled"}function KD(e){return["paused","disabled","inactive"].includes(e?.state)}function HD(e){return e?.state==="completed"}function Eh(e){return e?.next_run_timestamp??Ah(e?.next_run_at)}function Dh(e,t,a){try{return new Intl.DateTimeFormat(e||"en",t).format(a)}catch{return new Intl.DateTimeFormat("en",t).format(a)}}function QD(e,t,a){return!ur(e,0,23)||!ur(t,0,59)?null:Dh(a,{hour:"numeric",minute:"2-digit"},new Date(2001,0,1,Number(e),Number(t)))}function VD(e,t){return Dh(t,{weekday:"long"},new Date(2001,0,7+e))}function GD(e,t,a,n){let r=a!=null?{month:"short",day:"numeric",year:"numeric"}:{month:"short",day:"numeric"};return Dh(n,r,new Date(a??2e3,e-1,t))}function YD(e){let t=e.trim().split(/\s+/);if(t.length===5){let[a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===6&&DN(t[0])){let[,a,n,r,s,i]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:"*"}}if(t.length===7&&DN(t[0])){let[,a,n,r,s,i,o]=t;return{minute:a,hour:n,dayOfMonth:r,month:s,dayOfWeek:i,year:o}}return null}function DN(e){return/^0+$/.test(e)}function ur(e,t,a){if(!/^\d+$/.test(e))return!1;let n=Number(e);return n>=t&&n<=a}function JD(e){let t=/^\*\/(\d+)$/.exec(e);if(!t)return null;let a=Number(t[1]);return a>=1&&a<=59?a:null}function XD(e){let t=String(e||"").toUpperCase();return{SUN:"0",MON:"1",TUE:"2",WED:"3",THU:"4",FRI:"5",SAT:"6","MON-FRI":"1-5"}[t]||e}var ZD=8;function Mh(e){return e.run_id||e.thread_id||e.submitted_at||e.timestamp_source}function cd({runs:e=[]}){let t=R(),a=Array.isArray(e)?e:[],n=a.slice(0,ZD);if(!n.length)return l`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;let r=a.length-n.length,s=`+${Math.min(r,999)}`;return l`
    <div
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf",{shown:n.length,total:a.length})}
    >
      ${n.map(i=>l`
        <span
          key=${Mh(i)}
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
  `}function dd({runs:e=[],className:t=""}){let a=R(),n=qN(e,a);return n.total?l`
    <div className=${V("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]",t)}>
      <span className="text-iron-300">${n.totalText}</span>
      ${n.chips.map(r=>l`<span key=${r.key} className=${r.tone}>${r.text}</span>`)}
    </div>
  `:l`<span className=${V("text-[11px] text-iron-400",t)}>
      ${a("automations.table.noRuns")}
    </span>`}function IN({run:e,onOpenRun:t,onOpenLogs:a}){let n=R(),r=!!e.chat_path,s=Hc({threadId:e.thread_id,runId:e.run_id}),i=!!((e.thread_id||e.run_id)&&a);return l`
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
          <${D} name="chat" className="mr-1.5 h-4 w-4" />
          ${n("automations.detail.openRun")}
        <//>
        <${A}
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
  `}function md({label:e,value:t,tone:a}){return l`
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
  `}function KN({automation:e,isMutating:t=!1,onPauseAutomation:a,onResumeAutomation:n,onDeleteAutomation:r}){let s=R(),i=fe();if(!e)return l`
      <${I} className="p-4 sm:p-5">
        <${xe}
          boxed=${!1}
          title=${s("automations.detail.emptyTitle")}
          description=${s("automations.detail.emptyDescription")}
        />
      <//>
    `;let o=e.current_run,u=e.state==="paused",c=e.state==="active"||e.state==="scheduled",f=`${s(u?"missions.action.resume":"missions.action.pause")}: ${e.display_name}`,m=()=>{if(u){n?.(e.automation_id);return}c&&a?.(e.automation_id)},h=`${s("common.delete")}: ${e.display_name}`,b=()=>{window.confirm(h)&&r?.(e.automation_id)};return l`
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
                aria-label=${f}
                title=${f}
                disabled=${t}
                onClick=${m}
              >
                <${D} name=${u?"play":"pause"} className="h-4 w-4" />
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
              <${D} name="trash" className="h-4 w-4" />
            <//>
          </div>
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${md} label=${s("automations.detail.schedule")} value=${e.schedule_label} />
          <${md}
            label=${s("automations.detail.successRate")}
            value=${e.success_rate_label}
            tone=${e.has_failed_runs?"danger":"success"}
          />
          <${md} label=${s("automations.detail.lastCompleted")} value=${e.last_run_label} />
          <${md}
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
              <${cd} runs=${e.recent_runs} />
              <${dd} runs=${e.recent_runs} />
            </div>
          </div>

          ${e.recent_runs.length?l`
                <div>
                  ${e.recent_runs.map(y=>l`
                    <${IN}
                      key=${Mh(y)}
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
  `}var WD=["automations.empty.example1","automations.empty.example2","automations.empty.example3"];function eM({promptKey:e}){let t=R(),a=t(e),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>()=>clearTimeout(s.current),[]),l`
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
        <${D} name=${n?"check":"copy"} className="h-4 w-4" />
      </button>
    </li>
  `}function HN(){let e=R(),t=fe();return l`
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
            ${WD.map(a=>l`<${eM} key=${a} promptKey=${a} />`)}
          </ul>
        </div>

        <div className="mt-6">
          <${A} variant="primary" size="sm" onClick=${()=>t("/chat")}>
            <${D} name="chat" className="mr-1.5 h-4 w-4" />
            ${e("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `}function QN({automations:e,filter:t,onFilterChange:a,onRefresh:n,isRefreshing:r,isMutating:s,selectedAutomationId:i,onSelectAutomation:o,onPauseAutomation:u,onResumeAutomation:c,onDeleteAutomation:d}){let f=R(),m=UN(e,t),h=e.length>0,b=m.find(y=>y.automation_id===i)||m[0]||null;return l`
    <div className="space-y-5">
      <${I} className="p-4 sm:p-5">
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
              ${Th.map(y=>l`
                <button
                  key=${y.value}
                  type="button"
                  aria-pressed=${t===y.value}
                  onClick=${()=>a(y.value)}
                  className=${V("min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight",t===y.value?"bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]":"text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")}
                >
                  ${f(y.labelKey)}
                </button>
              `)}
            </div>
            <${A}
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
              <${I} className="overflow-hidden">
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
                      ${m.map(y=>{let w=y.automation_id===b?.automation_id;return l`
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
                                <${cd} runs=${y.recent_runs} />
                                <${dd} runs=${y.recent_runs} />
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
                automation=${b}
                isMutating=${s}
                onPauseAutomation=${u}
                onResumeAutomation=${c}
                onDeleteAutomation=${d}
              />
            </div>
          `:h?l`
              <${xe}
                title=${f("automations.empty.matchingTitle")}
                description=${f("automations.empty.matchingDescription")}
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
              className=${V(c,"transition-colors hover:border-white/20 hover:bg-white/[0.05]","focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",o&&"border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30")}
            >
              ${u}
            </button>
          `:l`<div key=${s.key} className=${c}>${u}</div>`})}
      </div>
    <//>
  `}function tM(e){return e==="active"||e==="scheduled"}function aM(e){return Number.isFinite(e)?e:null}function GN(e,t=Date.now()){let a=Array.isArray(e)?e:[],n=null;for(let r of a){if(!r||(r.has_running_run&&(n=n==null?5e3:Math.min(n,5e3)),!tM(r.state)))continue;let s=aM(r.next_run_timestamp);if(s==null)continue;let i=s-t,o=i<=0?2e3:i<3e4?Math.max(1e3,i+1200):null;o!=null&&(n=n==null?o:Math.min(n,o))}return n}var rM=50,sM=25;function YN(e=!1){let{t,lang:a}=pl(),n=J(),r=K({queryKey:["automations",{includeCompleted:e}],queryFn:()=>Ax({limit:rM,runLimit:sM,includeCompleted:e}),refetchInterval:3e4,refetchIntervalInBackground:!1}),s=p.default.useMemo(()=>PN(r.data,t,a),[r.data,t,a]),i=p.default.useMemo(()=>jN(s),[s]),o=p.default.useMemo(()=>GN(s),[s]);p.default.useEffect(()=>{if(o==null)return;let h=setTimeout(()=>{r.refetch()},o);return()=>clearTimeout(h)},[o,r.refetch]);let u=r.data?.scheduler_enabled!==!1,c=p.default.useCallback(()=>{n.invalidateQueries({queryKey:["automations"]})},[n]),d=Q({mutationFn:h=>Dx({automationId:h}),onSuccess:c}),f=Q({mutationFn:h=>Mx({automationId:h}),onSuccess:c}),m=Q({mutationFn:h=>Ox({automationId:h}),onSuccess:c});return{automations:s,summary:i,schedulerEnabled:u,isLoading:r.isLoading,isRefreshing:r.isFetching,isMutating:d.isPending||f.isPending||m.isPending,error:r.error||null,actionError:d.error||f.error||m.error||null,pauseAutomation:d.mutate,resumeAutomation:f.mutate,deleteAutomation:m.mutate,refetch:r.refetch}}var JN=["outbound-delivery","preferences"],XN=["outbound-delivery","targets"];function ZN(){let e=J(),t=K({queryKey:JN,queryFn:jx}),a=K({queryKey:XN,queryFn:Fx}),n=Q({mutationFn:({finalReplyTargetId:i})=>Bx({finalReplyTargetId:i}),onSuccess:i=>{e.setQueryData(JN,i),e.invalidateQueries({queryKey:XN})}}),r=p.default.useMemo(()=>a.data?.targets??[],[a.data]),s=p.default.useMemo(()=>r.filter(i=>i?.capabilities?.final_replies),[r]);return{preferences:t.data??null,targets:r,finalReplyTargets:s,currentTarget:t.data?.final_reply_target??null,currentStatus:t.data?.final_reply_target_status??"none_configured",isLoading:t.isLoading||a.isLoading,isRefreshing:t.isFetching||a.isFetching,isSaving:n.isPending,error:t.error||a.error||null,saveError:n.error||null,saveFinalReplyTarget:i=>n.mutateAsync({finalReplyTargetId:i}),refetch:()=>{t.refetch(),a.refetch()}}}function WN(){let e=R(),[t,a]=p.default.useState("all"),[n,r]=p.default.useState(null),i=YN(t==="completed"),o=ZN(),[u,c]=p.default.useState(!1),d=p.default.useRef(null);p.default.useEffect(()=>()=>clearTimeout(d.current),[]);let f=p.default.useCallback(()=>{c(!0),clearTimeout(d.current),d.current=setTimeout(()=>c(!1),1e3),i.refetch()},[i.refetch]),m=i.isRefreshing||u,h=i.error&&!i.isLoading&&i.automations.length===0;return p.default.useEffect(()=>{if(!i.automations.length){r(null);return}i.automations.some(y=>y.automation_id===n)||r(i.automations[0].automation_id)},[i.automations,n]),l`
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
                        ${[1,2,3].map(b=>l`<div
                              key=${b}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`)}
                      </div>
                    `:l`
                      <${QN}
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
  `}var e_={success:"border-mint/30 bg-mint/10 text-mint",error:"border-red-400/30 bg-red-500/10 text-red-200",info:"border-signal/30 bg-signal/10 text-signal"};function t_({result:e,onDismiss:t}){return p.default.useEffect(()=>{if(!e)return;let a=setTimeout(t,4e3);return()=>clearTimeout(a)},[e,t]),e?l`
    <div className=${["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",e_[e.type]||e_.info].join(" ")}>
      <${D}
        name=${e.type==="success"?"check":e.type==="error"?"close":"bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${e.message}</span>
      <button onClick=${t} className="shrink-0 opacity-70 hover:opacity-100">
        <${D} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `:null}var n_="/api/webchat/v2/channels/slack/setup";function r_(){return H(n_)}function s_(e){let t={installation_id:String(e.installation_id||"").trim(),team_id:String(e.team_id||"").trim(),api_app_id:String(e.api_app_id||"").trim(),user_id:a_(e.user_id),shared_subject_user_id:a_(e.shared_subject_user_id)},a=String(e.bot_token||"").trim(),n=String(e.signing_secret||"").trim();return a&&(t.bot_token=a),n&&(t.signing_secret=n),H(n_,{method:"PUT",body:JSON.stringify(t)})}function Oh(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}function a_(e){let t=String(e||"").trim();return t||null}var i_="/api/webchat/v2/channels/slack/allowed",iM="/api/webchat/v2/channels/slack/subjects";function o_(e=[]){return Array.from(new Set(e.map(t=>String(t||"").trim()).filter(Boolean))).sort()}function l_(){return H(i_)}function u_(){return H(iM)}function c_(e){let t=e.some(r=>typeof r!="string"),a=e.map(r=>typeof r=="string"?{channel_id:r}:{channel_id:r.channel_id,subject_user_id:r.subject_user_id}),n=t?{channels:a}:{channel_ids:a.map(r=>r.channel_id)};return H(i_,{method:"PUT",body:JSON.stringify(n)})}function d_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var m_=["slack-allowed-channels"];function p_({action:e}){let t=R(),a=J(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,u]=p.default.useState([]),c=lM(e,t),d=K({queryKey:m_,queryFn:l_}),f=K({queryKey:["slack-routable-subjects"],queryFn:u_}),m=f.data?.subjects||[],h=f_(m),b=f.isSuccess||f.isError,y=m.length>0;p.default.useEffect(()=>{d.data&&u(Lh(d.data.channels||[]))},[d.data]);let w=Q({mutationFn:({channels:C})=>c_(C),onSuccess:C=>{u(Lh(C.channels||[])),a.invalidateQueries({queryKey:m_}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["connectable-channels"]})}}),g=()=>{let C=n.trim();!C||!f.isSuccess||(u(_=>Lh([..._,{channel_id:C,subject_user_id:s}])),r(""))},v=C=>{u(_=>_.filter(T=>T.channel_id!==C))},x=(C,_)=>{u(T=>T.map(M=>M.channel_id===C?{...M,subject_user_id:_}:M))},$=()=>{w.mutate({channels:oM(o)})},S=f.isError&&o.some(C=>!C.subject_user_id);return l`
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
          ${!y&&l`<option value="">${c.noSubjectsLabel}</option>`}
          ${y&&l`<option value="">${c.autoSubjectLabel}</option>`}
          ${h.map(C=>l`
              <option key=${C.subject_user_id} value=${C.subject_user_id}>
                ${C.display_name}
              </option>
            `)}
        </select>
        <${A}
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
        ${o.map(C=>l`
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
                ${y?l`
                    <select
                      value=${C.subject_user_id}
                      onChange=${_=>x(C.channel_id,_.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${c.autoSubjectLabel}</option>
                      ${f_(m,C).map(_=>l`
                          <option key=${_.subject_user_id} value=${_.subject_user_id}>
                            ${_.display_name}
                          </option>
                        `)}
                    </select>
                  `:l`<span className="max-w-40 truncate text-xs text-iron-500">
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
        <${A}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${$}
          disabled=${!d.isSuccess||!b||w.isPending||S}
        >
          ${w.isPending?c.savingLabel:c.submitLabel}
        <//>
        ${w.isSuccess&&l`<p className="text-xs text-emerald-300">
          ${c.successMessage}
        </p>`}
        ${(d.isError||f.isError||w.isError)&&l`<p className="text-xs text-red-300">
          ${d_(w.error||d.error||f.error,c.errorMessage)}
        </p>`}
      </div>
    </div>
  `}function f_(e=[],t={}){let a=new Map;for(let r of e){let s=String(r.subject_user_id||"").trim();s&&a.set(s,{subject_user_id:s,display_name:r.display_name||s})}let n=String(t.subject_user_id||"").trim();return n&&!a.has(n)&&a.set(n,{subject_user_id:n,display_name:t.subject_display_name||n}),Array.from(a.values()).sort((r,s)=>r.display_name.localeCompare(s.display_name)||r.subject_user_id.localeCompare(s.subject_user_id))}function Lh(e=[]){let t=new Map;for(let a of e){let n=String(a.channel_id||"").trim();if(!n)continue;let r={channel_id:n,subject_user_id:String(a.subject_user_id||"").trim()},s=String(a.subject_display_name||"").trim();s&&(r.subject_display_name=s),t.set(n,r)}return o_(Array.from(t.keys())).map(a=>t.get(a))}function oM(e=[]){return e.map(t=>({channel_id:t.channel_id,subject_user_id:t.subject_user_id}))}function lM(e,t){return{title:e?.title||t("channels.slackAccessTitle"),instructions:e?.instructions||t("channels.slackAccessInstructions"),inputPlaceholder:e?.input_placeholder||e?.code_placeholder||"C0123456789",addLabel:t("channels.slackAccessAdd"),loadingMessage:t("channels.slackAccessLoading"),emptyMessage:t("channels.slackAccessEmpty"),submitLabel:e?.submit_label||t("channels.slackAccessSave"),savingLabel:t("channels.slackAccessSaving"),successMessage:e?.success_message||t("channels.slackAccessSuccess"),errorMessage:e?.error_message||t("channels.slackAccessError"),autoSubjectLabel:t("channels.slackAccessAutoSubject"),noSubjectsLabel:t("channels.slackAccessNoSubjects"),allowLabel:a=>t("channels.slackAccessAllow",{channelId:a})}}var Ph=["slack-setup"],Hr={installationId:{body:"Local IronClaw name for this Slack install. Choose one and keep it stable.",example:"Example: local-slack"},teamId:{body:"Slack workspace/team ID from the workspace that installed the app.",example:"Example: T0123456789"},appId:{body:"Slack app Basic Information > App Credentials.",example:"Example: A0123456789"},botUser:{body:"Optional Reborn user. Blank uses the current WebUI operator.",example:"Example: user:operator"},sharedSubject:{body:"Optional default team agent for shared channel turns. Usually blank.",example:"Example: user:slack-shared"},botToken:{body:"Slack app OAuth & Permissions > Bot User OAuth Token.",example:"Example: xoxb-..."},signingSecret:{body:"Slack app Basic Information > App Credentials > Signing Secret.",example:""}};function g_({action:e}){let t=K({queryKey:Ph,queryFn:r_}),a=t.data?.configured===!0;return l`
    <div className="space-y-3">
      <${uM} action=${e} setupQuery=${t} />
      ${a&&l`<${p_} action=${e} />`}
    </div>
  `}function uM({action:e,setupQuery:t}){let a=J(),[n,r]=p.default.useState(cM()),s=p.default.useRef(!1),i=p.default.useRef(!1),o=t.data,u=dM(e);p.default.useEffect(()=>{!o||s.current||i.current||(r(h_(o)),s.current=!0)},[o]);let c=Q({mutationFn:s_,onSuccess:h=>{i.current=!1,r(h_(h)),s.current=!0,a.setQueryData(Ph,h),a.invalidateQueries({queryKey:Ph}),a.invalidateQueries({queryKey:["slack-allowed-channels"]}),a.invalidateQueries({queryKey:["slack-routable-subjects"]}),a.invalidateQueries({queryKey:["connectable-channels"]}),a.invalidateQueries({queryKey:["extensions"]})}}),d=h=>b=>{i.current=!0,r(y=>({...y,[h]:b.target.value}))},f=()=>c.mutate(n),m=n.installation_id.trim()&&n.team_id.trim()&&n.api_app_id.trim()&&(o?.bot_token_configured||n.bot_token.trim())&&(o?.signing_secret_configured||n.signing_secret.trim());return l`
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
        ${ul("Installation ID",n.installation_id,d("installation_id"),"",Hr.installationId)}
        ${ul("Team ID",n.team_id,d("team_id"),"",Hr.teamId)}
        ${ul("App ID",n.api_app_id,d("api_app_id"),"",Hr.appId)}
        ${ul("Bot user",n.user_id,d("user_id"),"default operator",Hr.botUser)}
        ${ul("Shared subject",n.shared_subject_user_id,d("shared_subject_user_id"),"optional",Hr.sharedSubject)}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${v_("Bot token",n.bot_token,d("bot_token"),o?.bot_token_configured,Hr.botToken)}
        ${v_("Signing secret",n.signing_secret,d("signing_secret"),o?.signing_secret_configured,Hr.signingSecret)}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${A}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${f}
          disabled=${!m||c.isPending}
        >
          ${c.isPending?"Saving...":u.submitLabel}
        <//>
        ${t.isError&&l`<p className="text-xs text-red-300">
          ${Oh(t.error,u.errorMessage)}
        </p>`}
        ${c.isError&&l`<p className="text-xs text-red-300">
          ${Oh(c.error,u.errorMessage)}
        </p>`}
        ${c.isSuccess&&l`<p className="text-xs text-emerald-300">${u.successMessage}</p>`}
      </div>
    </div>
  `}function h_(e){return{installation_id:e.installation_id||"",team_id:e.team_id||"",api_app_id:e.api_app_id||"",user_id:e.user_id||"",shared_subject_user_id:e.shared_subject_user_id||"",bot_token:"",signing_secret:""}}function cM(){return{installation_id:"",team_id:"",api_app_id:"",user_id:"",shared_subject_user_id:"",bot_token:"",signing_secret:""}}function ul(e,t,a,n="",r=null){return l`
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
  `:null}function dM(e){return{title:"Slack setup",instructions:e?.instructions||"Configure the Slack app before assigning channels.",submitLabel:"Save setup",successMessage:"Slack setup saved.",errorMessage:"Slack setup update failed."}}var Uh={wasm_tool:"WASM Tool",wasm_channel:"Channel",channel:"Channel",mcp_server:"MCP Server",first_party:"First-party",system:"System",channel_relay:"Relay"};function Qr(e){return e==="wasm_channel"||e==="channel"}var b_={active:"success",ready:"success",pairing_required:"warning",pairing:"warning",auth_required:"warning",setup_required:"muted",failed:"danger",installed:"muted"},x_={active:"active",ready:"ready",pairing_required:"pairing",pairing:"pairing",auth_required:"auth needed",setup_required:"setup needed",failed:"failed",installed:"installed"};function $_(e){let t=w_(e);return!e?.package_ref||t==="active"||t==="ready"?null:t==="auth_required"||t==="setup_required"?"configure":e?.kind==="wasm_channel"||Qr(e?.kind)&&(t==="pairing_required"||t==="pairing")?null:"activate"}function w_(e){return e?.onboarding_state||e?.onboardingState||e?.activation_status||e?.activationStatus||(e?.active?"active":"installed")}function jh(e){let t=w_(e);return t==="active"||t==="ready"}function S_({extension:e,secrets:t=[],fields:a=[]}={}){return jh(e)||a.length>0||t.length===0?!1:t.every(n=>n.provided)}var N_="flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",__="mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]",k_="mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]",R_="mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3",C_="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]",mM="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";function E_(e){return e.package_ref?.id||""}function fM({actions:e,isBusy:t}){let a=R(),[n,r]=p.default.useState(!1),s=p.default.useRef(null);return p.default.useEffect(()=>{if(!n)return;let i=o=>{s.current&&!s.current.contains(o.target)&&r(!1)};return document.addEventListener("mousedown",i),()=>document.removeEventListener("mousedown",i)},[n]),l`
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
  `}function T_({items:e}){return!e||e.length===0?null:l`
    <div className="mt-3 flex flex-wrap gap-1">
      ${e.map(t=>l`<span key=${t} className=${mM}>${t}</span>`)}
    </div>
  `}function vi({ext:e,onActivate:t,onConfigure:a,onRemove:n,isBusy:r}){let s=R(),i=e.onboarding_state||e.activation_status||(e.active?"active":"installed"),o=b_[i]||"muted",u=s(`extensions.state.${i}`)||x_[i]||i,c=s(`extensions.kind.${e.kind}`)||Uh[e.kind]||e.kind,d=e.display_name||E_(e),f=!!e.package_ref,m=e.tools||[],[h,b]=p.default.useState(!1),w=(i==="setup_required"||i==="auth_required"?e.onboarding?.credential_instructions||e.onboarding?.credential_next_step:e.onboarding?.credential_next_step||e.onboarding?.credential_instructions)||null,g={packageRef:e.package_ref,displayName:d,active:e.active,activationStatus:e.activation_status,onboardingState:e.onboarding_state},v=[],x=[],$=$_(e);$==="configure"?v.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),run:()=>a(g)}):$==="activate"&&v.push({id:"activate",label:"Activate",run:()=>t(g)}),f&&(e.needs_setup||e.has_auth)&&$!=="configure"&&x.push({id:"configure",label:e.authenticated?s("extensions.reconfigure"):s("extensions.configure"),icon:"settings",run:()=>a(g)}),f&&Qr(e.kind)&&(i==="setup_required"||i==="failed")&&x.push({id:"setup",label:"Setup",icon:"settings",run:()=>a(g)}),f&&Qr(e.kind)&&(i==="active"||i==="ready"||i==="pairing_required"||i==="pairing")&&x.push({id:"reconfigure",label:"Reconfigure",icon:"settings",run:()=>a(g)}),f&&x.push({id:"remove",label:s("common.remove")||"Remove",icon:"trash",danger:!0,run:()=>n(g)});let S=v[0];return l`
    <div className=${N_}>
      <div className="flex items-start gap-2">
        <${q} tone=${o} label=${u} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${d}
        </span>
        ${x.length>0&&l`<${fM} actions=${x} isBusy=${r} />`}
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

      ${w&&l`
        <div className="mt-2 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]">
          ${w}
        </div>
      `}

      <div className=${R_}>
        ${m.length>0?l`
              <button
                type="button"
                aria-expanded=${h?"true":"false"}
                onClick=${()=>b(C=>!C)}
                className=${C_}
              >
                <${D} name="layers" className="h-3.5 w-3.5" />
                <span>${m.length===1?s("extensions.oneCapability"):s("extensions.pluralCapabilities",{count:m.length})}</span>
                <${D}
                  name="chevron"
                  className=${["h-3 w-3",h?"rotate-180":""].join(" ")}
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

      ${h&&l`<${T_} items=${m} />`}
    </div>
  `}function Vr({entry:e,onInstall:t,isBusy:a,statusLabel:n}){let r=R(),s=r(`extensions.kind.${e.kind}`)||Uh[e.kind]||e.kind,i=e.display_name||E_(e),o=!!(e.package_ref&&t),u=e.keywords||[],[c,d]=p.default.useState(!1);return l`
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
        ${u.length>0?l`
              <button
                type="button"
                aria-expanded=${c?"true":"false"}
                onClick=${()=>d(f=>!f)}
                className=${C_}
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
          <${A}
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

      ${c&&l`<${T_} items=${u} />`}
    </div>
  `}function A_(){return H("/api/webchat/v2/extensions")}function D_(){return H("/api/webchat/v2/extensions/registry")}function M_(e){return H("/api/webchat/v2/extensions/install",{method:"POST",body:JSON.stringify({package_ref:e})})}function O_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(cl(e))}/activate`,{method:"POST"})}function L_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(cl(e))}/remove`,{method:"POST"})}function P_(e){return H(`/api/webchat/v2/extensions/${encodeURIComponent(cl(e))}/setup`)}function U_(e,t,a){return Gx(cl(e),{action:"submit",payload:{secrets:t,fields:a}})}function j_(e,t){let a=t?.setup||{},n=new Date(Date.now()+10*60*1e3).toISOString();return H(`/api/webchat/v2/extensions/${encodeURIComponent(cl(e))}/setup/oauth/start`,{method:"POST",body:JSON.stringify({provider:t.provider,account_label:a.account_label||`${t.provider} credential`,scopes:a.scopes||[],expires_at:n,invocation_id:a.invocation_id})})}function F_(){return Promise.resolve({requests:[]})}function B_(){return Promise.resolve({success:!1,message:"Pairing requires a v2 pairing endpoint."})}function cl(e){let t=typeof e=="string"?e:e?.id;if(!t)throw new Error("Extension package_ref is required");return t}var pM=2e3,hM=10*60*1e3;function gi(e){return e?.package_ref?.id||null}function Fh(e){return e?.display_name||gi(e)||""}function z_(e,t,a){return gi(t)||`${e}:${Fh(t)||"unknown"}:${a}`}function vM(e,t){return e.installed!==t.installed?e.installed?-1:1:Fh(e.entry||e.extension).localeCompare(Fh(t.entry||t.extension))}function q_(){let e=J(),t=K({queryKey:["gateway-status-extensions"],queryFn:Js,staleTime:1e4}),a=K({queryKey:["extensions"],queryFn:A_}),n=K({queryKey:["extension-registry"],queryFn:D_}),r=K({queryKey:["connectable-channels"],queryFn:Bc}),s=p.default.useCallback(()=>{e.invalidateQueries({queryKey:["extensions"]}),e.invalidateQueries({queryKey:["extension-registry"]}),e.invalidateQueries({queryKey:["gateway-status-extensions"]}),e.invalidateQueries({queryKey:["connectable-channels"]})},[e]),[i,o]=p.default.useState(null),u=p.default.useCallback(()=>o(null),[]),c=Q({mutationFn:({packageRef:k})=>M_(k),onSuccess:(k,{displayName:z})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${z||"Extension"} installed`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):o({type:"error",message:k.message||"Install failed"}),s()},onError:k=>{o({type:"error",message:k.message}),s()}}),d=Q({mutationFn:({packageRef:k})=>O_(k),onSuccess:(k,{displayName:z})=>{k.success?(o({type:"success",message:k.message||k.instructions||`${z||"Extension"} activated`}),k.auth_url&&window.open(k.auth_url,"_blank","noopener,noreferrer")):k.auth_url?(window.open(k.auth_url,"_blank","noopener,noreferrer"),o({type:"info",message:"Opening authentication\u2026"})):k.awaiting_token?o({type:"info",message:"Configuration required"}):o({type:"error",message:k.message||"Activation failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),f=Q({mutationFn:({packageRef:k})=>L_(k),onSuccess:(k,{displayName:z})=>{k.success?o({type:"success",message:`${z||"Extension"} removed`}):o({type:"error",message:k.message||"Remove failed"}),s()},onError:k=>{o({type:"error",message:k.message})}}),m=t.data||{},h=a.data?.extensions||[],b=n.data?.entries||[],y=r.data?.channels||[],w=new Map(h.map(k=>[gi(k),k]).filter(([k])=>!!k)),g=new Set(b.map(k=>gi(k)).filter(Boolean)),v=[...b.map((k,z)=>{let Z=gi(k),re=Z&&w.get(Z)||null;return{id:z_("registry",k,z),installed:!!(re||k.installed),entry:k,extension:re}}),...h.filter(k=>{let z=gi(k);return!z||!g.has(z)}).map((k,z)=>({id:z_("installed",k,z),installed:!0,entry:null,extension:k}))].sort(vM),x=k=>Qr(k.kind),$=h.filter(x),S=h.filter(k=>k.kind==="mcp_server"),C=h.filter(k=>!x(k)&&k.kind!=="mcp_server"),_=b.filter(k=>x(k)&&!k.installed),T=b.filter(k=>k.kind==="mcp_server"&&!k.installed),M=b.filter(k=>k.kind!=="mcp_server"&&!x(k)&&!k.installed),O=a.isLoading||n.isLoading,U=c.isPending||d.isPending||f.isPending;return{status:m,extensions:h,channels:$,mcpServers:S,tools:C,channelRegistry:_,mcpRegistry:T,toolRegistry:M,registry:b,catalogEntries:v,connectableChannels:y,isLoading:O,isBusy:U,actionResult:i,clearResult:u,install:c.mutate,activate:d.mutate,remove:f.mutate,invalidate:s}}function I_(e){let t=K({queryKey:["extension-setup",e?.id||e],queryFn:()=>P_(e),enabled:!!e});return{secrets:t.data?.secrets||[],fields:t.data?.fields||[],onboarding:t.data?.onboarding||null,isLoading:t.isLoading,error:t.error}}function K_(e,t){let a=J(),n=e?.id||e;return Q({mutationFn:({secrets:r,fields:s})=>U_(e,r,s),onSuccess:r=>{a.invalidateQueries({queryKey:["extensions"]}),a.invalidateQueries({queryKey:["extension-setup",n]}),t&&t(r)}})}function H_(e){let t=J(),a=e?.id||e,n=p.default.useRef(null),r=p.default.useCallback(()=>{n.current&&(window.clearInterval(n.current),n.current=null)},[]),s=p.default.useCallback(()=>{t.invalidateQueries({queryKey:["extensions"]}),t.invalidateQueries({queryKey:["extension-registry"]}),t.invalidateQueries({queryKey:["extension-setup",a]})},[a,t]),i=p.default.useCallback(()=>{let u=t.getQueryData(["extension-setup",a]);if(u?.secrets?.length>0&&u.secrets.every(m=>m.provided))return!0;let d=(t.getQueryData(["extensions"])?.extensions||[]).find(m=>m.package_ref?.id===a),f=d?.onboarding_state||d?.activation_status||(d?.active?"active":null);return f==="active"||f==="ready"},[a,t]),o=p.default.useCallback(u=>{r();let c=Date.now();n.current=window.setInterval(()=>{s(),(i()||u&&u.closed||Date.now()-c>hM)&&(r(),s())},pM)},[r,s,i]);return p.default.useEffect(()=>r,[r]),Q({mutationFn:({secret:u,popup:c})=>j_(e,u).then(d=>({res:d,popup:c})),onSuccess:({res:u,popup:c})=>{let d=c;u.authorization_url&&c&&!c.closed?c.location.href=u.authorization_url:u.authorization_url?d=window.open(u.authorization_url,"_blank","noopener,noreferrer"):c&&!c.closed&&c.close(),s(),d&&o(d)},onError:(u,c)=>{r();let d=c?.popup;d&&!d.closed&&d.close()}})}function Q_(e,t={}){let a=K({queryKey:["pairing",e],queryFn:()=>F_(e),enabled:!!e&&t.enabled!==!1,refetchInterval:5e3}),n=J(),r=Q({mutationFn:({code:s})=>B_(e,s),onSuccess:()=>{n.invalidateQueries({queryKey:["pairing",e]}),n.invalidateQueries({queryKey:["extensions"]})}});return{requests:a.data?.requests||[],isLoading:a.isLoading,approve:r.mutate,isApproving:r.isPending,result:r.isSuccess?r.data:null,error:r.isError?r.error:null}}function V_(e,t){return e?.payload?.error||e?.payload?.message||e?.message||t}var gM={title:"pairing.title",instructions:"pairing.instructions",placeholder:"pairing.placeholder",action:"pairing.approve",success:"pairing.success",error:"pairing.error",empty:"pairing.none"};function G_({channel:e,redeemFn:t,i18nKeys:a=gM,queryKeys:n,copy:r,showPendingRequests:s=!0}){let i=R(),o=typeof t=="function",u=Q_(e,{enabled:!o}),c=J(),[d,f]=p.default.useState(""),m=yM(i,a,r),h=Q({mutationFn:({code:S})=>t(e,S),onSuccess:()=>{f("");for(let S of n||[["pairing",e],["extensions"]])c.invalidateQueries({queryKey:S})}}),b=p.default.useCallback(S=>u.approve({code:S}),[u.approve]),y=p.default.useCallback(()=>{let S=d.trim();S&&(o?h.mutate({code:S}):(u.approve({code:S}),f("")))},[o,d,u.approve,h]),w=o?[]:u.requests,g=o?!1:u.isLoading,v=o?h.isPending:u.isApproving,x=o?h.isSuccess?h.data:null:u.result,$=o?h.isError?h.error:null:u.error;return g?l`
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
        <${A}
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
      ${$&&l`<p className="mb-3 text-xs text-red-300">
        ${V_($,m.error)}
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
  `}function yM(e,t,a){return{title:a?.title||e(t.title),instructions:a?.instructions||e(t.instructions),placeholder:a?.input_placeholder||a?.code_placeholder||e(t.placeholder),action:a?.submit_label||e(t.action),success:a?.success_message||e(t.success),error:a?.error_message||e(t.error)}}function fd(e){return e.package_ref?.id||""}function Y_(e){return fd(e)==="slack"}function X_(e){return e?.channel==="slack"&&e.strategy==="admin_managed_channels"}function Z_(e){return e?.channel==="slack"&&e.strategy==="inbound_proof_code"}function bM(e){let t=e||[],a=[t.find(X_),t.find(Z_)].filter(Boolean);if(a.length>0)return a;let n=t.find(r=>r.channel==="slack");return n?[n]:[]}function J_({slackConnectAction:e,slackConnectActions:t}){let n=(t||(e?[e]:[])).map(r=>X_(r)?l`<${g_} action=${r.action} />`:Z_(r)?l`<${Oc} action=${r.action} />`:null).filter(Boolean);return n.length>0?l`<div className="space-y-3">${n}</div>`:null}function W_({status:e,channels:t,connectableChannels:a,channelRegistry:n,onActivate:r,onConfigure:s,onRemove:i,onInstall:o,isBusy:u}){let c=R(),d=t||[],f=e.enabled_channels||[],m=bM(a),h=d.some(Y_),b=m.length>0&&!h;return l`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${c("channels.builtIn")}
        </h3>
        <${yi}
          name="Web Gateway"
          description=${c("channels.webGatewayDesc")||"Browser-based chat with SSE streaming"}
          enabled=${!0}
          detail=${"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)}
        />
        <${yi}
          name="HTTP Webhook"
          description=${c("channels.httpWebhookDesc")||"Inbound webhook endpoint for external integrations"}
          enabled=${f.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${yi}
          name="CLI"
          description=${c("channels.cliDesc")||"Terminal interface with TUI or simple REPL"}
          enabled=${f.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${yi}
          name="REPL"
          description=${c("channels.replDesc")||"Minimal read-eval-print loop for testing"}
          enabled=${f.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${b&&l`
          <${yi}
            name=${c("channels.slack")||"Slack"}
            description=${c("channels.slackDesc")||"Tenant app channel for DMs and app mentions"}
            enabled=${!1}
            statusLabel="setup"
            statusTone="muted"
            detail=${c("channels.slackDetail")||"Tenant Slack app install"}
          >
            <${J_}
              slackConnectActions=${m}
            />
          </${yi}>
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
                <div key=${fd(y)} className="flex flex-col gap-3">
                  <${vi}
                    ext=${y}
                    onActivate=${r}
                    onConfigure=${s}
                    onRemove=${i}
                    isBusy=${u}
                  />
                  ${Y_(y)&&l`<${J_}
                    slackConnectActions=${m}
                  />`}
                  ${(y.onboarding_state==="pairing_required"||y.onboarding_state==="pairing")&&l` <${G_} channel=${fd(y)} /> `}
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
                <${Vr}
                  key=${fd(y)}
                  entry=${y}
                  onInstall=${o}
                  isBusy=${u}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function yi({name:e,description:t,enabled:a,detail:n,children:r,statusLabel:s=a?"on":"off",statusTone:i=a?"success":"muted"}){return l`
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
  `}function ek({extension:e,onActivate:t,onClose:a,onSaved:n}){let r=R(),s=e?.displayName||e?.packageRef?.id||"Extension",{secrets:i=[],fields:o=[],onboarding:u,isLoading:c,error:d}=I_(e?.packageRef),[f,m]=p.default.useState({}),[h,b]=p.default.useState({}),y=H_(e?.packageRef),w=K_(e?.packageRef,_=>{_.success!==!1&&(n&&n(_),a())}),g=p.default.useCallback(()=>{let _={};for(let[T,M]of Object.entries(f)){let O=(M||"").trim();O&&(_[T]=O)}w.mutate({secrets:_,fields:h})},[f,h,w]),v=p.default.useCallback(_=>{let T=window.open("about:blank","_blank","width=600,height=600");T&&(T.opener=null),y.mutate({secret:_,popup:T})},[y]),$=i.filter(_=>(_.setup?.kind||"manual_token")==="manual_token").length>0||o.length>0,S=jh(e),C=S_({extension:e,secrets:i,fields:o});return c?l`
      <${pd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <div className="space-y-3">
          ${[1,2].map(_=>l`<div
                key=${_}
                className="v2-skeleton h-10 w-full rounded-md"
              />`)}
        </div>
      <//>
    `:d?l`
      <${pd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-red-200">
          ${r("extensions.loadFailed")||"Failed to load setup:"} ${d.message}
        </p>
      <//>
    `:i.length===0&&o.length===0?l`
      <${pd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
        <p className="text-sm text-iron-300">
          ${r("extensions.noConfigRequired")||"No configuration required for this extension."}
        </p>
      <//>
    `:l`
    <${pd} onClose=${a} title=${r("extensions.configureName").replace("{name}",s)}>
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
                value=${f[_.name]||""}
                onChange=${T=>m(M=>({...M,[_.name]:T.target.value}))}
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
                value=${h[_.name]||""}
                onChange=${T=>b(M=>({...M,[_.name]:T.target.value}))}
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
        ${C&&l`
        <${A}
          variant="primary"
          onClick=${()=>t?.(e)}
        >
          Activate
        <//>
        `}
        ${$&&l`
        <${A}
          variant=${C?"secondary":"primary"}
          onClick=${g}
          disabled=${w.isPending}
        >
          ${w.isPending?"Saving\u2026":r("common.save")||"Save"}
        <//>
        `}
      </div>
    <//>
  `}function pd({onClose:e,title:t,children:a}){return p.default.useEffect(()=>{let n=r=>{r.key==="Escape"&&e()};return window.addEventListener("keydown",n),()=>window.removeEventListener("keydown",n)},[e]),l`
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
  `}function tk(e){return e.package_ref?.id||""}function ak({mcpServers:e,mcpRegistry:t,onActivate:a,onConfigure:n,onRemove:r,onInstall:s,isBusy:i}){let o=R();return e.length===0&&t.length===0?l`
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
                <${vi}
                  key=${tk(u)}
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
                <${Vr}
                  key=${tk(u)}
                  entry=${u}
                  onInstall=${s}
                  isBusy=${i}
                />
              `)}
          </div>
        </div>
      `}
    </div>
  `}function xM(e){return e?.package_ref?.id||""}function $M(e){return e.entry||e.extension||{}}function nk({catalogEntries:e,onInstall:t,onActivate:a,onConfigure:n,onRemove:r,isBusy:s}){let i=R(),[o,u]=p.default.useState(""),c=o.trim().toLowerCase(),d=c?e.filter(y=>{let w=$M(y);return(w.display_name||xM(w)).toLowerCase().includes(c)||(w.description||"").toLowerCase().includes(c)||(w.keywords||[]).some(g=>g.toLowerCase().includes(c))}):e,f=d.filter(y=>y.installed&&y.extension),m=d.filter(y=>y.installed&&!y.extension&&y.entry),h=f.length+m.length,b=d.filter(y=>!y.installed&&y.entry);return e.length===0?l`
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
                  ${f.map(y=>l`
                      <${vi}
                        key=${y.id}
                        ext=${y.extension||y.entry}
                        onActivate=${a}
                        onConfigure=${n}
                        onRemove=${r}
                        isBusy=${s}
                      />
                    `)}
                  ${m.map(y=>l`
                      <${Vr}
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
                  className=${["mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",h>0?"mt-6":""].join(" ")}
                >
                  ${i("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${b.map(y=>l`
                      <${Vr}
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
  `}function Bh(){let{tab:e="registry"}=it(),[t,a]=p.default.useState(null),{status:n,channels:r,mcpServers:s,channelRegistry:i,mcpRegistry:o,catalogEntries:u,connectableChannels:c,isLoading:d,isBusy:f,actionResult:m,clearResult:h,install:b,activate:y,remove:w,invalidate:g}=q_(),v=p.default.useCallback(_=>a(_),[]),x=p.default.useCallback(()=>a(null),[]),$=p.default.useCallback(()=>g(),[g]),S=p.default.useCallback(_=>{_&&(y(_),a(null))},[y]);if(d)return l`
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
    `;if(e==="installed")return l`<${ot} to="/extensions/registry" replace />`;let C={channels:l`<${W_}
      status=${n}
      channels=${r}
      connectableChannels=${c}
      channelRegistry=${i}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${w}
      onInstall=${b}
      isBusy=${f}
    />`,mcp:l`<${ak}
      mcpServers=${s}
      mcpRegistry=${o}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${w}
      onInstall=${b}
      isBusy=${f}
    />`,registry:l`<${nk}
      catalogEntries=${u}
      onInstall=${b}
      onActivate=${y}
      onConfigure=${v}
      onRemove=${w}
      isBusy=${f}
    />`};return C[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${t_} result=${m} onDismiss=${h} />
          ${C[e]}
        </div>
      </div>

      ${t&&l`
        <${ek}
          extension=${t}
          onActivate=${S}
          onClose=${x}
          onSaved=${$}
        />
      `}
    </div>
  `:l`<${ot} to="/extensions/registry" replace />`}var rk=[{groupKey:"settings.group.embeddings",fields:[{key:"embeddings.enabled",labelKey:"settings.field.embeddingsEnabled",descKey:"settings.field.embeddingsEnabledDesc",type:"boolean"},{key:"embeddings.provider",labelKey:"settings.field.embeddingsProvider",descKey:"settings.field.embeddingsProviderDesc",type:"select",options:["openai","nearai"]},{key:"embeddings.model",labelKey:"settings.field.embeddingsModel",descKey:"settings.field.embeddingsModelDesc",type:"text"}]},{groupKey:"settings.group.sampling",fields:[{key:"temperature",labelKey:"settings.field.temperature",descKey:"settings.field.temperatureDesc",type:"float",min:0,max:2,step:.1}]}],sk=[{groupKey:"settings.group.core",fields:[{key:"agent.name",labelKey:"settings.field.agentName",descKey:"settings.field.agentNameDesc",type:"text"},{key:"agent.max_parallel_jobs",labelKey:"settings.field.maxParallelJobs",descKey:"settings.field.maxParallelJobsDesc",type:"number"},{key:"agent.job_timeout_secs",labelKey:"settings.field.jobTimeout",descKey:"settings.field.jobTimeoutDesc",type:"number"},{key:"agent.max_tool_iterations",labelKey:"settings.field.maxToolIterations",descKey:"settings.field.maxToolIterationsDesc",type:"number"},{key:"agent.use_planning",labelKey:"settings.field.planning",descKey:"settings.field.planningDesc",type:"boolean"},{key:"agent.default_timezone",labelKey:"settings.field.timezone",descKey:"settings.field.timezoneDesc",type:"text"},{key:"agent.session_idle_timeout_secs",labelKey:"settings.field.sessionIdleTimeout",descKey:"settings.field.sessionIdleTimeoutDesc",type:"number"},{key:"agent.stuck_threshold_secs",labelKey:"settings.field.stuckThreshold",descKey:"settings.field.stuckThresholdDesc",type:"number"},{key:"agent.max_repair_attempts",labelKey:"settings.field.maxRepairAttempts",descKey:"settings.field.maxRepairAttemptsDesc",type:"number"},{key:"agent.max_cost_per_day_cents",labelKey:"settings.field.dailyCostLimit",descKey:"settings.field.dailyCostLimitDesc",type:"number",min:0},{key:"agent.max_actions_per_hour",labelKey:"settings.field.actionsPerHour",descKey:"settings.field.actionsPerHourDesc",type:"number",min:0},{key:"agent.allow_local_tools",labelKey:"settings.field.allowLocalTools",descKey:"settings.field.allowLocalToolsDesc",type:"boolean"}]},{groupKey:"settings.group.heartbeat",fields:[{key:"heartbeat.enabled",labelKey:"settings.field.heartbeatEnabled",descKey:"settings.field.heartbeatEnabledDesc",type:"boolean"},{key:"heartbeat.interval_secs",labelKey:"settings.field.heartbeatInterval",descKey:"settings.field.heartbeatIntervalDesc",type:"number"},{key:"heartbeat.notify_channel",labelKey:"settings.field.heartbeatNotifyChannel",descKey:"settings.field.heartbeatNotifyChannelDesc",type:"text"},{key:"heartbeat.notify_user",labelKey:"settings.field.heartbeatNotifyUser",descKey:"settings.field.heartbeatNotifyUserDesc",type:"text"},{key:"heartbeat.quiet_hours_start",labelKey:"settings.field.quietHoursStart",descKey:"settings.field.quietHoursStartDesc",type:"number",min:0,max:23},{key:"heartbeat.quiet_hours_end",labelKey:"settings.field.quietHoursEnd",descKey:"settings.field.quietHoursEndDesc",type:"number",min:0,max:23},{key:"heartbeat.timezone",labelKey:"settings.field.heartbeatTimezone",descKey:"settings.field.heartbeatTimezoneDesc",type:"text"}]},{groupKey:"settings.group.sandbox",fields:[{key:"sandbox.enabled",labelKey:"settings.field.sandboxEnabled",descKey:"settings.field.sandboxEnabledDesc",type:"boolean"},{key:"sandbox.policy",labelKey:"settings.field.sandboxPolicy",descKey:"settings.field.sandboxPolicyDesc",type:"select",options:["readonly","workspace_write","full_access"]},{key:"sandbox.timeout_secs",labelKey:"settings.field.sandboxTimeout",descKey:"settings.field.sandboxTimeoutDesc",type:"number",min:0},{key:"sandbox.memory_limit_mb",labelKey:"settings.field.sandboxMemoryLimit",descKey:"settings.field.sandboxMemoryLimitDesc",type:"number",min:0},{key:"sandbox.image",labelKey:"settings.field.sandboxImage",descKey:"settings.field.sandboxImageDesc",type:"text"}]},{groupKey:"settings.group.routines",fields:[{key:"routines.max_concurrent",labelKey:"settings.field.routinesMaxConcurrent",descKey:"settings.field.routinesMaxConcurrentDesc",type:"number",min:0},{key:"routines.default_cooldown_secs",labelKey:"settings.field.routinesDefaultCooldown",descKey:"settings.field.routinesDefaultCooldownDesc",type:"number",min:0}]},{groupKey:"settings.group.safety",fields:[{key:"safety.max_output_length",labelKey:"settings.field.safetyMaxOutput",descKey:"settings.field.safetyMaxOutputDesc",type:"number",min:0},{key:"safety.injection_check_enabled",labelKey:"settings.field.safetyInjectionCheck",descKey:"settings.field.safetyInjectionCheckDesc",type:"boolean"}]},{groupKey:"settings.group.skills",fields:[{key:"skills.max_active",labelKey:"settings.field.skillsMaxActive",descKey:"settings.field.skillsMaxActiveDesc",type:"number",min:0},{key:"skills.max_context_tokens",labelKey:"settings.field.skillsMaxContextTokens",descKey:"settings.field.skillsMaxContextTokensDesc",type:"number",min:0}]},{groupKey:"settings.group.search",fields:[{key:"search.fusion_strategy",labelKey:"settings.field.fusionStrategy",descKey:"settings.field.fusionStrategyDesc",type:"select",options:["rrf","weighted"]}]}],ik=[{groupKey:"settings.group.gateway",fields:[{key:"channels.gateway_host",labelKey:"settings.field.gatewayHost",descKey:"settings.field.gatewayHostDesc",type:"text"},{key:"channels.gateway_port",labelKey:"settings.field.gatewayPort",descKey:"settings.field.gatewayPortDesc",type:"number"}]},{groupKey:"settings.group.tunnel",fields:[{key:"tunnel.provider",labelKey:"settings.field.tunnelProvider",descKey:"settings.field.tunnelProviderDesc",type:"select",options:["ngrok","cloudflare","tailscale","custom"]},{key:"tunnel.public_url",labelKey:"settings.field.tunnelPublicUrl",descKey:"settings.field.tunnelPublicUrlDesc",type:"text"}]}],zh=new Set(["embeddings.enabled","embeddings.provider","embeddings.model","tunnel.provider","tunnel.public_url","gateway.rate_limit","gateway.max_connections"]);function ok(e){return String(e||"").trim().toLowerCase()}function lk(e){if(e==null)return"";if(Array.isArray(e))return e.map(lk).join(" ");if(typeof e=="object")try{return JSON.stringify(e)}catch{return""}return String(e)}function tt(e,t){let a=ok(e);return a?t.map(lk).join(" ").toLowerCase().includes(a):!0}function bi(e,t,a,n){let r=ok(a);return r?e.map(s=>{let i=s.groupKey?n(s.groupKey):"",o=s.fields.filter(u=>tt(r,[i,u.key,u.labelKey?n(u.labelKey):u.label,u.descKey?n(u.descKey):u.description,t[u.key]]));return{...s,fields:o}}).filter(s=>s.fields.length>0):e}function wM({visible:e}){let t=R();return e?l`
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
  `}function NM({field:e,value:t,onSave:a,isSaved:n}){let r=R(),[s,i]=p.default.useState(""),o=e.labelKey?r(e.labelKey):e.label||"",u=e.descKey?r(e.descKey):e.description||"";p.default.useEffect(()=>{e.type!=="boolean"&&i(t!=null?String(t):"")},[t,e.type]);let c=p.default.useCallback(d=>{if(d==="")a(e.key,null);else if(e.type==="number"){let f=parseInt(d,10);isNaN(f)||a(e.key,f)}else if(e.type==="float"){let f=parseFloat(d);isNaN(f)||a(e.key,f)}else a(e.key,d)},[e.key,e.type,a]);return l`
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
  `}function xi({group:e,groupKey:t,fields:a,settings:n,onSave:r,savedKeys:s}){let i=R(),o=t?i(t):e||"";return l`
    <${te} className="p-4 sm:p-6">
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
  `}function Rt({query:e}){let t=R();return l`
    <${te} padding="lg">
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
  `}function uk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return l`<${_M} />`;let i=bi(sk,e,r,s);return i.length===0?l`<${Rt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${xi}
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
  `}function ck(){let e=K({queryKey:["gateway-status-settings"],queryFn:Js,staleTime:1e4}),t=K({queryKey:["extensions"],queryFn:I$}),a=K({queryKey:["extension-registry"],queryFn:K$}),n=e.data||{},r=t.data?.extensions||[],s=a.data?.entries||[],i=r.filter(f=>f.kind==="wasm_channel"||f.kind==="channel"),o=s.filter(f=>(f.kind==="wasm_channel"||f.kind==="channel")&&!f.installed),u=r.filter(f=>f.kind==="mcp_server"),c=s.filter(f=>f.kind==="mcp_server"&&!f.installed),d=e.isLoading||t.isLoading;return{status:n,channels:i,channelRegistry:o,mcpServers:u,mcpRegistry:c,extensions:r,isLoading:d}}function kM({name:e,description:t,enabled:a,detail:n}){let r=R();return l`
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
  `}function dk({channel:e,registryEntry:t}){let a=R(),n=t?.display_name||e?.name||t?.name||a("common.unknown"),r=t?.description||e?.description||"",s=!!e,i=e?.onboarding_state||"setup_required",o={ready:"positive",auth_required:"warning",pairing_required:"warning",setup_required:"muted"},u={ready:a("channels.ready"),auth_required:a("channels.authNeeded"),pairing_required:a("channels.pairing"),setup_required:a("channels.setup")};return l`
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
  `}function RM(e,t){let a=e.enabled_channels||[];return[{id:"web",name:t("channels.webGateway"),description:t("channels.webGatewayDesc"),enabled:!0,detail:"SSE: "+(e.sse_connections||0)+" \xB7 WS: "+(e.ws_connections||0)},{id:"http",name:t("channels.httpWebhook"),description:t("channels.httpWebhookDesc"),enabled:a.includes("http"),detail:"ENABLE_HTTP=true"},{id:"cli",name:t("channels.cli"),description:t("channels.cliDesc"),enabled:a.includes("cli"),detail:"ironclaw run --cli"},{id:"repl",name:t("channels.repl"),description:t("channels.replDesc"),enabled:a.includes("repl"),detail:"ironclaw run --repl"}]}function CM({status:e,channels:t,channelRegistry:a,mcpServers:n,mcpRegistry:r,searchQuery:s,t:i}){let o=RM(e,i).filter(b=>tt(s,[i("channels.builtIn"),b.id,b.name,b.description,b.detail])),u=new Set(t.map(b=>b.name)),c=t.filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description,b.onboarding_state])),d=a.filter(b=>!u.has(b.name)).filter(b=>tt(s,[i("channels.messaging"),b.name,b.display_name,b.description])),f=new Set(n.map(b=>b.name)),m=n.filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description,b.active?i("channels.active"):i("channels.inactive")])),h=r.filter(b=>!f.has(b.name)).filter(b=>tt(s,[i("channels.mcpServers"),b.name,b.display_name,b.description]));return{builtInChannels:o,visibleChannels:c,availableRegistry:d,visibleMcpServers:m,availableMcp:h}}function mk({searchQuery:e=""}){let t=R(),{status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,isLoading:o}=ck();if(o)return l`
      <div className="space-y-5">
        <${te} padding="md">
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
    `;let{builtInChannels:u,visibleChannels:c,availableRegistry:d,visibleMcpServers:f,availableMcp:m}=CM({status:a,channels:n,channelRegistry:r,mcpServers:s,mcpRegistry:i,searchQuery:e,t});return u.length===0&&c.length===0&&d.length===0&&f.length===0&&m.length===0?l`<${Rt} query=${e} />`:l`
    <div className="space-y-5">
      ${u.length>0&&l`
      <${te} padding="md">
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
        <${te} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${c.map(h=>l`
              <${dk}
                key=${h.name}
                channel=${h}
                registryEntry=${r.find(b=>b.name===h.name)}
              />
            `)}
          ${d.map(h=>l`
              <${dk} key=${h.name} registryEntry=${h} />
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
  `}function fk({provider:e,activeProviderId:t,selectedModel:a,builtinOverrides:n,isBusy:r,onUse:s,onConfigure:i,onDelete:o,onNearaiLogin:u,onNearaiWallet:c,onCodexLogin:d,loginBusy:f}){let m=R(),h=e.id===t,b=zr(e,n),y=ei(e,n),w=nw(e,n,t,a),g=wc(e,n),v=rw(e),x=m(g==="api_key"?"llm.missingApiKey":g==="base_url"?"llm.missingBaseUrl":"llm.notConfigured"),[$,S]=p.default.useState(h),C=p.default.useCallback(()=>S(Ee=>!Ee),[]);p.default.useEffect(()=>{S(h)},[h]);let _=b?l`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${Xo(e.adapter)} · ${w||e.default_model||m("llm.none")}
      </span>`:l`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${x}
      </span>`,T=e.id==="nearai"||e.id==="openai_codex",M=e.api_key_set===!0||e.has_api_key===!0,O=e.builtin?e.id==="nearai"&&v&&!M?m("llm.addApiKey"):m("llm.configure"):m("common.edit"),U=v&&e.builtin?l`
          <${A}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${r}
            onClick=${()=>i(e)}
          >
            ${O}
          <//>
        `:null,k=!h&&e.id==="nearai"?l`
          ${U}
          <${A} type="button" variant="secondary" size="sm" disabled=${f} onClick=${c}>
            ${m("onboarding.nearWallet")}
          <//>
          <${A} type="button" variant="secondary" size="sm" disabled=${f} onClick=${()=>u("github")}>
            GitHub
          <//>
          <${A} type="button" variant="secondary" size="sm" disabled=${f} onClick=${()=>u("google")}>
            Google
          <//>
        `:!h&&e.id==="openai_codex"?l`
          <${A} type="button" variant="secondary" size="sm" disabled=${f} onClick=${d}>
            ${m("onboarding.codexSignIn")}
          <//>
        `:null,Z=!h&&b&&(!T||e.id==="nearai"&&e.has_api_key===!0)?l`
        <${A}
          type="button"
          variant="primary"
          size="sm"
          disabled=${r}
          onClick=${()=>s(e)}
        >
          ${m("llm.use")}
        <//>
      `:null,re=b?null:l`
        <${A}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${r}
          onClick=${()=>i(e)}
        >
          ${m(g==="api_key"?"llm.addApiKey":"llm.configure")}
        <//>
      `,me=h?null:Z||(T?k:re),pe=!T&&(e.builtin&&e.id!=="bedrock"||!e.builtin)||e.id==="nearai"&&v;return l`
    <${te}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${e.id}
      className=${["transition-colors",h?"border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]":$?"border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]":""].join(" ")}
    >
      <div className="flex w-full items-stretch hover:bg-[var(--v2-surface-soft)]">
        <button
          type="button"
          aria-expanded=${$?"true":"false"}
          aria-label=${m($?"llm.collapseDetails":"llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${C}
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
            ${h&&l`<${q} tone="positive" label=${m("llm.active")} size="sm" />`}
            ${e.builtin&&!h&&l`<${q} tone="muted" label=${m("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${_}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${me}
          <button
            type="button"
            onClick=${C}
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
              <div className="mt-1 truncate">${Xo(e.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${y||m("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${m("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${w||m("llm.none")}</div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${pe&&l`
              <${A}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${r}
                onClick=${()=>i(e)}
              >
                ${O}
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
                ${m("common.delete")}
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
  `}function pk({settings:e,gatewayStatus:t,searchQuery:a=""}){let n=R(),r=Gc({settings:e,gatewayStatus:t,searchQuery:a,t:n}),s=r.providerState,i=Yc(),o=i.nearaiBusy||i.codexBusy;if(a&&r.filteredProviders.length===0)return l`<${Rt} query=${a} />`;let u=sw(r.filteredProviders,s.builtinOverrides,s.activeProviderId);return l`
    <${te} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${n("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${n("llm.providersDesc")}</p>
        </div>
        <${A} type="button" variant="secondary" size="sm" className="gap-2" onClick=${()=>r.openDialog(null)}>
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

      <${Vc} login=${i} />

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
                      ${d.map(f=>l`
                          <${fk}
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

      <${Qc}
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
  `}function hk({settings:e,gatewayStatus:t,onSave:a,savedKeys:n,isLoading:r,searchQuery:s=""}){let i=R(),{activeProviderId:o,selectedModel:u,providers:c,hasActiveProvider:d}=ti({settings:e,gatewayStatus:t});if(r)return l`<${AM} />`;let f=d?o:"",m=c.find(g=>g.id===o),h=d&&(u||m?.default_model||e.selected_model)||"",b=bi(rk,e,s,i),y=tt(s,[i("inference.provider"),i("inference.backend"),f,i("inference.model"),h]),w=tt(s,[i("llm.providers"),i("llm.providersDesc"),i("llm.addProvider"),"llm","provider","openai","anthropic","ollama","near"]);return!y&&!w&&b.length===0?l`<${Rt} query=${s} />`:l`
    <div className="space-y-5">
      ${y&&l`
      <${te} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${i("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${i("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${f||i("inference.none")}</span>
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

      ${w&&l`
        <${pk}
          settings=${e}
          gatewayStatus=${t}
          searchQuery=${s}
        />
      `}

      ${b.map(g=>l`
            <${xi}
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
  `}function AM(){return l`
    <div className="space-y-5">
      <${te} padding="md">
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
            <${te} key=${e} padding="md">
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
  `}function vk({searchQuery:e=""}){let t=R(),{lang:a,setLang:n}=pl(),r=hl.find(i=>i.code===a)||hl[0],s=hl.filter(i=>tt(e,[i.code,i.name,i.native]));return s.length===0?l`<${Rt} query=${e} />`:l`
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
  `}function gk({settings:e,onSave:t,savedKeys:a,isLoading:n,searchQuery:r=""}){let s=R();if(n)return l`
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
    `;let i=bi(ik,e,r,s);return i.length===0?l`<${Rt} query=${r} />`:l`
    <div className="space-y-5">
      ${i.map(o=>l`
            <${xi}
              key=${o.groupKey}
              groupKey=${o.groupKey}
              fields=${o.fields}
              settings=${e}
              onSave=${t}
              savedKeys=${a}
            />
          `)}
    </div>
  `}function yk(){let e=R(),[t,a]=p.default.useState(!1),n=p.default.useCallback(()=>a(!0),[]),r=p.default.useCallback(()=>a(!1),[]),s=p.default.useCallback(()=>a(!1),[]);return{restartEnabled:!1,unavailableReason:e("settings.restartUnavailable"),isRestarting:!1,progressLabel:"",error:null,message:null,confirmOpen:t,openConfirm:n,closeConfirm:r,confirmRestart:s}}function bk({visible:e,gatewayStatus:t,gatewayStatusQuery:a}){let n=R(),r=yk({gatewayStatus:t,gatewayStatusQuery:a});return e?l`
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

        <${A}
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

    <${oi}
      open=${r.confirmOpen}
      onClose=${r.closeConfirm}
      title=${n("restart.title")}
      size="sm"
    >
      <${li} className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          ${n("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          ${n("restart.warning")}
        </div>
      <//>
      <${ui}>
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
  `:null}function xk(){let e=J(),t=K({queryKey:["skills"],queryFn:H$}),a=Q({mutationFn:V$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),n=Q({mutationFn:Y$,onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),r=Q({mutationFn:({name:c,content:d})=>G$(c,{content:d}),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),s=Q({mutationFn:({name:c,enabled:d})=>J$(c,d),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),i=Q({mutationFn:c=>X$(c),onSuccess:()=>{e.invalidateQueries({queryKey:["skills"]})}}),o=t.data?.skills||[],u=t.data?.auto_activate_learned!==!1;return{skills:o,query:t,autoActivateLearned:u,fetchSkillContent:Q$,installSkill:a.mutateAsync,removeSkill:n.mutateAsync,updateSkill:r.mutateAsync,setSkillAutoActivate:s.mutateAsync,setAutoActivateLearned:i.mutateAsync,isInstalling:a.isPending,isRemoving:n.isPending,isUpdating:r.isPending,isSettingAutoActivate:s.isPending,isSettingAutoActivateLearned:i.isPending}}function $k({skill:e,onEdit:t,onRemove:a,onUpdate:n,onSetAutoActivate:r,isRemoving:s,isUpdating:i,isSettingAutoActivate:o}){let u=R(),c=e.name||e.id,d=e.trust||e.trust_level||"installed",f=e.source_kind||"installed",m=!!e.can_edit,h=!!e.can_delete,b=e.auto_activate!==!1,[y,w]=p.default.useState(!1),[g,v]=p.default.useState(""),[x,$]=p.default.useState(""),[S,C]=p.default.useState(!1);p.default.useEffect(()=>{y||(v(""),$(""))},[y]);let _=p.default.useCallback(async()=>{C(!0),$("");try{let M=await t(c);v(M?.content||""),w(!0)}catch(M){$(M.message||u("skills.contentLoadFailed"))}finally{C(!1)}},[c,t,u]),T=p.default.useCallback(async()=>{(await n(c,g))?.success&&w(!1)},[g,c,n]);return l`
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
              tone=${f==="system"?"positive":"muted"}
              label=${u(`skills.source.${f}`)}
              size="sm"
            />
            ${e.version&&l`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${e.version}</span>`}
          </div>

          ${e.description&&l`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${e.description}</div>`}

          ${y?l`
                <div className="mt-3">
                  <${Dc}
                    rows=${12}
                    value=${g}
                    className="font-mono text-xs leading-5"
                    onInput=${M=>v(M.currentTarget.value)}
                  />
                </div>
              `:l`<${DM} skill=${e} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${m&&!y&&l`
            <${A}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${i||S}
              title=${u("skills.edit")}
              onClick=${_}
            >
              <${D} name="file" className="h-4 w-4" />
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
              <${D} name="close" className="h-4 w-4" />
              ${u("skills.cancel")}
            <//>
            <${A}
              type="button"
              variant="primary"
              size="sm"
              disabled=${i}
              onClick=${T}
            >
              <${D} name="check" className="h-4 w-4" />
              ${u(i?"skills.saving":"skills.save")}
            <//>
          `}
          ${m&&!y&&l`
            <${A}
              type="button"
              variant=${b?"secondary":"ghost"}
              size="sm"
              disabled=${o}
              title=${u(b?"skills.autoActivateOnTitle":"skills.autoActivateOffTitle")}
              onClick=${()=>r(c,!b)}
            >
              <${D} name=${b?"check":"close"} className="h-4 w-4" />
              ${u(b?"skills.autoActivateOnLabel":"skills.autoActivateOffLabel")}
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
              <${D} name="trash" className="h-4 w-4" />
              ${u("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${x&&l`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${x}</p>`}
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
        ${e.has_requirements&&l`<${qh}>requirements.txt<//>`}
        ${e.has_scripts&&l`<${qh}>scripts/<//>`}
        ${e.install_source_url&&l`<${qh}>${t("skills.imported")}<//>`}
      </div>
    `}
  `}function qh({children:e}){return l`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${e}
    </span>
  `}function wk({onInstall:e,isInstalling:t}){let a=R(),[n,r]=p.default.useState(""),[s,i]=p.default.useState(""),[o,u]=p.default.useState({name:"",content:""}),[c,d]=p.default.useState(""),[f,m]=p.default.useState(""),h=p.default.useCallback((y,w)=>{u(g=>!g[y]||!w.trim()?g:{...g,[y]:""})},[]),b=p.default.useCallback(async()=>{let y=MM({name:n,content:s}),w=OM(y,a);if(w.name||w.content){u(w),d(""),m("");return}u({name:"",content:""}),d(""),m("");try{let g=await e(y);if(!g?.success){d(g?.message||a("skills.installFailed"));return}r(""),i(""),m(g.message||a("skills.installedSuccess",{name:y.name}))}catch(g){d(g.message||a("skills.installFailed"))}},[s,n,e,a]);return l`
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

      <${_n} label=${a("skills.name")} error=${o.name} required>
        <${Mt}
          size="sm"
          error=${!!o.name}
          aria-invalid=${o.name?"true":void 0}
          value=${n}
          placeholder=${a("skills.namePlaceholder")}
          onInput=${y=>{let w=y.currentTarget.value;r(w),h("name",w)}}
        />
      <//>

      <${_n}
        className="mt-3"
        label=${a("skills.content")}
        error=${o.content}
        hint=${a("skills.contentHint")}
        required
      >
        <${Dc}
          rows=${5}
          error=${!!o.content}
          aria-invalid=${o.content?"true":void 0}
          value=${s}
          placeholder=${a("skills.contentPlaceholder")}
          onInput=${y=>{let w=y.currentTarget.value;i(w),h("content",w)}}
        />
      <//>

      ${c&&l`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${c}</p>`}
      ${f&&l`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${f}</p>`}

      <div className="mt-4 flex justify-end">
        <${A} type="button" size="sm" disabled=${t} onClick=${b}>
          <${D} name="upload" className="h-4 w-4" />
          ${a(t?"skills.installing":"skills.install")}
        <//>
      </div>
    <//>
  `}function MM({name:e,content:t}){let a={name:e.trim()};return t.trim()&&(a.content=t.trim()),a}function OM(e,t){return{name:e.name?"":t("skills.nameRequired"),content:e.content?"":t("skills.contentRequired")}}function Sk({searchQuery:e=""}){let t=R(),{skills:a,query:n,autoActivateLearned:r,fetchSkillContent:s,installSkill:i,removeSkill:o,updateSkill:u,setSkillAutoActivate:c,setAutoActivateLearned:d,isInstalling:f,isRemoving:m,isUpdating:h,isSettingAutoActivate:b,isSettingAutoActivateLearned:y}=xk(),[w,g]=p.default.useState(""),[v,x]=p.default.useState(""),$=p.default.useCallback(async M=>{if(window.confirm(t("skills.confirmDelete",{name:M}))){g(""),x("");try{let O=await o(M);if(!O?.success){g(O?.message||t("skills.removeFailed"));return}x(O.message||t("skills.removed",{name:M}))}catch(O){g(O.message||t("skills.removeFailed"))}}},[o,t]),S=p.default.useCallback(async(M,O)=>{if(!O.trim())return g(t("skills.contentRequired")),x(""),{success:!1,message:t("skills.contentRequired")};g(""),x("");try{let U=await u({name:M,content:O});return U?.success?(x(U.message||t("skills.updated",{name:M})),U):(g(U?.message||t("skills.updateFailed")),U)}catch(U){let k=U.message||t("skills.updateFailed");return g(k),{success:!1,message:k}}},[t,u]),C=p.default.useCallback(async(M,O)=>{g(""),x("");try{let U=await c({name:M,enabled:O});if(!U?.success){g(U?.message||t("skills.updateFailed"));return}x(U.message)}catch(U){g(U.message||t("skills.updateFailed"))}},[c,t]),_=p.default.useCallback(async M=>{g(""),x("");try{let O=await d(M);if(!O?.success){g(O?.message||t("skills.updateFailed"));return}x(O.message)}catch(O){g(O.message||t("skills.updateFailed"))}},[d,t]),T;if(n.isLoading)T=l`
      <${te} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3].map(M=>l`
            <div key=${M} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;else if(n.error)T=l`
      <${te} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad",{message:n.error.message})}</p>
        <//>
    `;else{let M=a.filter(U=>tt(e,[U.name,U.id,U.description,U.keywords,U.trust_level,U.source_kind,U.version])),O=UM(M);a.length===0?T=l`
        <${te} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `:M.length===0?T=l`<${Rt} query=${e} />`:T=l`
        <div id="skills-list">
          ${O.map(U=>l`
              <${PM}
                key=${U.id}
                title=${t(U.labelKey)}
                skills=${U.skills}
                onEdit=${s}
                onRemove=${$}
                onUpdate=${S}
                onSetAutoActivate=${C}
                isRemoving=${m}
                isUpdating=${h}
                isSettingAutoActivate=${b}
              />
            `)}
        </div>
      `}return l`
    <div className="space-y-4">
      <${LM}
        enabled=${r}
        isSaving=${y}
        onToggle=${_}
      />
      <${wk} onInstall=${i} isInstalling=${f} />
      <${jM} error=${w} result=${v} />
      ${T}
    </div>
  `}function LM({enabled:e,isSaving:t,onToggle:a}){let n=R();return l`
    <${te} padding="md" style=${e?void 0:{background:"var(--v2-danger-soft)"}}>
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
    <${te} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${e}
      </h3>
      ${t.map(c=>l`
          <${$k}
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
  `}function hd(e,t="Request failed"){if(e&&e.success===!1)throw new Error(e.message||t);return e}function Nk(){let e=J(),t=K({queryKey:["settings-tools"],queryFn:z$}),a=t.data?.tools||[],[n,r]=p.default.useState({}),s=Q({mutationFn:async({name:o,state:u})=>hd(await q$(o,u),"Save failed"),onSuccess:(o,{name:u,state:c})=>{e.setQueryData(["settings-tools"],d=>{if(!d)return d;let f=o?.tool;return{...d,tools:d.tools.map(m=>m.name===u?{...m,state:c,...f||{}}:m)}}),r(d=>({...d,[u]:!0})),setTimeout(()=>r(d=>({...d,[u]:!1})),2e3)}}),i=p.default.useCallback((o,u)=>s.mutate({name:o,state:u}),[s]);return{tools:a,query:t,setPermission:i,savedTools:n,error:s.error}}var vd="agent.auto_approve_tools";function FM({visible:e}){let t=R();return e?l`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `:null}function BM({checked:e,disabled:t=!1,label:a,onChange:n}){return l`
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
  `}function Ih({settings:e,onSave:t,savedKeys:a,isLoading:n}){let r=R(),s=r("settings.field.autoApproveEligibleTools"),i=e?.[vd]===!0||e?.[vd]==="true";return l`
    <${te} padding="md" className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
          ${s}
        </h3>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          ${r("settings.field.autoApproveEligibleToolsDesc")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <${FM} visible=${a?.[vd]} />
        <${BM}
          checked=${i}
          disabled=${n}
          label=${s}
          onChange=${o=>t(vd,o)}
        />
      </div>
    <//>
  `}function zM({tool:e,onPermissionChange:t,isSaved:a}){let n=R(),r=[{value:"default",label:n("tools.followDefault"),tone:"neutral"},{value:"always_allow",label:n("tools.alwaysAllow"),tone:"positive"},{value:"ask_each_time",label:n("tools.askEachTime"),tone:"warning"},{value:"disabled",label:n("tools.disabled"),tone:"danger"}],s={default:n("tools.sourceDefault"),global:n("tools.sourceGlobal"),override:n("tools.sourceOverride")},i=e.locked,o=r.find(f=>f.value===e.state)||r[1],u=e.effective_source||"default",c=u==="override"?e.state:"default",d=u==="default"&&e.state===e.default_state;return l`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${i&&l`<${D}
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
        ${i?l`<${q} tone=${o.tone} label=${o.label} size="sm" />`:l`
              <select
                value=${c}
                onChange=${f=>t(e.name,f.target.value)}
                aria-label=${n("tools.permissionFor",{name:e.name})}
                className="v2-select h-8 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5 font-mono text-xs text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
              >
                ${r.map(f=>l`<option key=${f.value} value=${f.value}>
                      ${f.label}
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
  `}function _k({settings:e={},onSave:t=()=>{},savedKeys:a={},isLoading:n=!1,searchQuery:r=""}){let s=R(),{tools:i,query:o,setPermission:u,savedTools:c}=Nk();if(o.isLoading)return l`
      <div className="space-y-4">
        <${Ih}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${te} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1,2,3,4,5].map(f=>l`
              <div
                key=${f}
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
        <${Ih}
          settings=${e}
          onSave=${t}
          savedKeys=${a}
          isLoading=${n}
        />
        <${te} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">
            ${s("tools.failedLoad",{message:o.error.message})}
          </p>
        <//>
      </div>
    `;let d=i.filter(f=>tt(r,[f.name,f.description,f.state,f.default_state,f.effective_source,f.locked?s("tools.disabled"):""]));return l`
    <div className="space-y-4">
      <${Ih}
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

      <${te} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${s("tools.permissions")}
        </h3>
        ${d.length===0?l`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${s("tools.noMatch")}
            </p>`:d.map(f=>l`
                  <${zM}
                    key=${f.name}
                    tool=${f}
                    onPermissionChange=${u}
                    isSaved=${c[f.name]}
                  />
                `)}
      <//>
    </div>
  `}function kk(e){return(Number(e)||0).toFixed(2)}function qM(e){let t=Number(e)||0;return`${t>=0?"+":""}${t.toFixed(2)}`}function Rk(e,t){if(!e)return t("traceCommons.never");let a=new Date(e);return Number.isNaN(a.getTime())?t("traceCommons.never"):a.toLocaleString()}function Gr({label:e,value:t,description:a}){return l`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${e}</div>
        ${a&&l`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${a}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${t}</div>
    </div>
  `}function Ck({searchQuery:e=""}){let t=R(),{credits:a,query:n,authorize:r}=_c();if(!tt(e,["trace commons","credits",t("settings.traceCommons"),t("traceCommons.title")]))return l`<${Rt} query=${e} />`;let s;if(n.isLoading)s=l`
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
        <${Gr}
          label=${t("traceCommons.enrollment")}
          value=${a.enrolled?t("traceCommons.enrolled"):t("traceCommons.notEnrolled")}
        />
        <${Gr}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${kk(a.pending_credit)}
        />
        <${Gr}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${kk(a.final_credit)}
        />
        <${Gr}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${qM(a.delayed_credit_delta)}
        />
        <${Gr}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue",{submitted:a.submissions_submitted||0,accepted:a.submissions_accepted||0,total:a.submissions_total||0})}
        />
        <${Gr}
          label=${t("traceCommons.lastSubmission")}
          value=${Rk(a.last_submission_at,t)}
        />
        <${Gr}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${Rk(a.last_credit_sync_at,t)}
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
  `}function Ek(){let e=J(),t=K({queryKey:["admin-users"],queryFn:ew,retry:!1}),a=t.data?.users||[],n=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),r=Q({mutationFn:tw,onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})}),s=Q({mutationFn:({id:i,payload:o})=>aw(i,o),onSuccess:()=>e.invalidateQueries({queryKey:["admin-users"]})});return{users:a,query:t,isForbidden:n,createUser:r.mutate,updateUser:(i,o)=>s.mutate({id:i,payload:o}),createError:r.error,isCreating:r.isPending}}function IM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[u,c]=p.default.useState("member"),[d,f]=p.default.useState(!1),m=h=>{h.preventDefault(),r.trim()&&e({display_name:r.trim(),email:i.trim()||void 0,role:u},{onSuccess:()=>{s(""),o(""),f(!1)}})};return d?l`
    <${te} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${n("users.newUser")}
      </h3>
      <form onSubmit=${m} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${_n} label=${n("users.displayName")} htmlFor="user-name">
            <${Mt}
              id="user-name"
              type="text"
              value=${r}
              onChange=${h=>s(h.target.value)}
              required
            />
          <//>
          <${_n} label=${n("users.email")} htmlFor="user-email">
            <${Mt}
              id="user-email"
              type="email"
              value=${i}
              onChange=${h=>o(h.target.value)}
            />
          <//>
        </div>
        <${_n} label=${n("users.role")} htmlFor="user-role">
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
            onClick=${()=>f(!1)}
            >${n("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `:l`
      <${A} variant="secondary" onClick=${()=>f(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("users.addUser")}
      <//>
    `}function KM({user:e}){let t=R(),a=e.status==="active"?"positive":"danger",n=e.role==="admin"?"accent":"muted";return l`
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
  `}function Tk({searchQuery:e=""}){let t=R(),{users:a,query:n,isForbidden:r,createUser:s,createError:i,isCreating:o}=Ek();if(n.isLoading)return l`
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
      <${te} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad",{message:n.error.message})}
        </p>
      <//>
    `;let u=a.filter(c=>tt(e,[c.id,c.display_name,c.email,c.role,c.status,c.last_active]));return l`
    <div className="space-y-5">
      <${IM}
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
            </p>`:u.map(c=>l`<${KM} key=${c.id} user=${c} />`)}
      <//>
    </div>
  `}function Ak(){let e=J(),t=K({queryKey:["settings-export"],queryFn:D$,staleTime:3e4}),a=t.data?.settings||{},[n,r]=p.default.useState({}),[s,i]=p.default.useState(!1),o=Q({mutationFn:async({key:f,value:m})=>hd(await Kp(f,m),"Save failed"),onSuccess:(f,{key:m,value:h})=>{e.setQueryData(["settings-export"],b=>{if(!b)return b;let y={...b,settings:{...b.settings}};return h==null?delete y.settings[m]:y.settings[m]=h,y}),r(b=>({...b,[m]:!0})),setTimeout(()=>r(b=>({...b,[m]:!1})),2e3),zh.has(m)&&i(!0),m==="agent.auto_approve_tools"&&e.invalidateQueries({queryKey:["settings-tools"]})}}),u=p.default.useCallback((f,m)=>o.mutate({key:f,value:m}),[o]),c=Q({mutationFn:M$,onSuccess:(f,m)=>{e.invalidateQueries({queryKey:["settings-export"]});let h=Object.keys(m?.settings||{});h.includes("agent.auto_approve_tools")&&e.invalidateQueries({queryKey:["settings-tools"]}),h.some(b=>zh.has(b))&&i(!0)}}),d=p.default.useCallback(f=>c.mutateAsync(f),[c]);return{settings:a,query:t,save:u,savedKeys:n,needsRestart:s,importSettings:d,isImporting:c.isPending,saveError:o.error||c.error}}function Kh(){let e=R(),{tab:t}=it(),{gatewayStatus:a,gatewayStatusQuery:n,isAdmin:r=!1}=wa(),s=r?"inference":"language",i=t||s,{settings:o,query:u,save:c,savedKeys:d,needsRestart:f,saveError:m}=Ak(),[h,b]=p.default.useState("");p.default.useEffect(()=>{b("")},[i]);let y=u.isLoading,w={inference:l`<${hk}
      settings=${o}
      gatewayStatus=${a}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,agent:l`<${uk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,channels:l`<${mk} searchQuery=${h} />`,networking:l`<${gk}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,tools:l`<${_k}
      settings=${o}
      onSave=${c}
      savedKeys=${d}
      isLoading=${y}
      searchQuery=${h}
    />`,skills:l`<${Sk} searchQuery=${h} />`,traces:l`<${Ck} searchQuery=${h} />`,users:l`<${Tk} searchQuery=${h} />`,language:l`<${vk} searchQuery=${h} />`},g=C=>C==="users"||C==="inference",v=C=>Object.prototype.hasOwnProperty.call(w,C),x=Object.keys(w).filter(C=>r||!g(C)),S=v(s)&&x.includes(s)?s:x[0]||"language";return!v(i)||!r&&g(i)?l`<${ot} to=${`/settings/${S}`} replace />`:l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${f&&l`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${bk}
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
  `}var Hh=Object.freeze({todo:!0});function Dk(){return Promise.resolve({users:[],total:0,...Hh})}function Mk(e){return Promise.resolve(null)}function Ok(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Lk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Pk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Uk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function jk(e){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Fk(e,t){return Promise.resolve({success:!1,message:"TODO: requires v2 admin endpoint"})}function Bk(){return Promise.resolve({total_users:0,active_users:0,suspended_users:0,admin_users:0,total_jobs:0,llm_calls:0,total_cost_usd:0,active_jobs:0,uptime_seconds:0,recent_users:[],...Hh})}function zk(e="day",t){return Promise.resolve({entries:[],...Hh})}function qk(){return K({queryKey:["admin","usage-summary"],queryFn:Bk,refetchInterval:3e4})}function gd(e="day",t){return K({queryKey:["admin","usage",e,t],queryFn:()=>zk(e,t),refetchInterval:3e4})}function $i(){let e=J(),t=K({queryKey:["admin","users"],queryFn:Dk,refetchInterval:1e4}),a=t.data,n=Array.isArray(a)?a:a?.users||[],r=t.error?.message?.includes("403")||t.error?.message?.includes("Forbidden"),s=()=>e.invalidateQueries({queryKey:["admin","users"]}),i=Q({mutationFn:Ok,onSuccess:s}),o=Q({mutationFn:({id:m,payload:h})=>Lk(m,h),onSuccess:s}),u=Q({mutationFn:m=>Pk(m),onSuccess:s}),c=Q({mutationFn:m=>Uk(m),onSuccess:s}),d=Q({mutationFn:m=>jk(m),onSuccess:s}),f=Q({mutationFn:({userId:m,name:h})=>Fk(m,h)});return{users:n,query:t,isForbidden:r,createUser:i.mutateAsync,isCreating:i.isPending,createError:i.error,updateUser:(m,h)=>o.mutateAsync({id:m,payload:h}),deleteUser:u.mutateAsync,suspendUser:c.mutateAsync,activateUser:d.mutateAsync,createToken:(m,h)=>f.mutateAsync({userId:m,name:h}),newToken:f.data,clearToken:()=>f.reset()}}function Ik(e){return K({queryKey:["admin","user",e],queryFn:()=>Mk(e),enabled:!!e,refetchInterval:1e4})}function Xa(e){return e==null||e===0?"0":e>=1e6?(e/1e6).toFixed(1)+"M":e>=1e3?(e/1e3).toFixed(1)+"K":String(e)}function Aa(e){if(e==null)return"$0.00";let t=parseFloat(e);return isNaN(t)?"$0.00":"$"+t.toFixed(2)}function Kk(e){if(!e)return"0s";let t=Math.floor(e/86400),a=Math.floor(e%86400/3600),n=Math.floor(e%3600/60);return t>0?`${t}d ${a}h`:a>0?`${a}h ${n}m`:`${n}m`}function dr(e){if(!e)return"Never";let t=(Date.now()-new Date(e).getTime())/1e3;return t<0||t<60?"Just now":t<3600?Math.floor(t/60)+"m ago":t<86400?Math.floor(t/3600)+"h ago":t<2592e3?Math.floor(t/86400)+"d ago":new Date(e).toLocaleDateString()}function wi(e){return e?e.length>12?e.slice(0,12)+"\u2026":e:""}function Si(e){return e==="active"?"success":e==="suspended"?"danger":"muted"}function Ni(e){return e==="admin"?"signal":"muted"}function Hk(e){let t=e.length,a=e.filter(s=>s.status==="active").length,n=e.filter(s=>s.status==="suspended").length,r=e.filter(s=>s.role==="admin").length;return{total:t,active:a,suspended:n,admins:r}}function Qk(e,{search:t="",filter:a="all"}){let n=e;if(a==="active"?n=n.filter(r=>r.status==="active"):a==="suspended"?n=n.filter(r=>r.status==="suspended"):a==="admin"&&(n=n.filter(r=>r.role==="admin")),t.trim()){let r=t.toLowerCase();n=n.filter(s=>s.display_name&&s.display_name.toLowerCase().includes(r)||s.email&&s.email.toLowerCase().includes(r)||s.id&&s.id.toLowerCase().includes(r))}return n}function Vk(e){let t={};for(let a of e)t[a.user_id]||(t[a.user_id]={user_id:a.user_id,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.user_id].calls+=a.call_count||0,t[a.user_id].input_tokens+=a.input_tokens||0,t[a.user_id].output_tokens+=a.output_tokens||0,t[a.user_id].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Gk(e){let t={};for(let a of e)t[a.model]||(t[a.model]={model:a.model,calls:0,input_tokens:0,output_tokens:0,cost:0}),t[a.model].calls+=a.call_count||0,t[a.model].input_tokens+=a.input_tokens||0,t[a.model].output_tokens+=a.output_tokens||0,t[a.model].cost+=parseFloat(a.total_cost)||0;return Object.values(t).sort((a,n)=>n.cost-a.cost)}function Yk(e){return e.reduce((t,a)=>({calls:t.calls+a.calls,input_tokens:t.input_tokens+a.input_tokens,output_tokens:t.output_tokens+a.output_tokens,cost:t.cost+a.cost}),{calls:0,input_tokens:0,output_tokens:0,cost:0})}function HM({users:e,onSelectUser:t}){let a=R(),n=[...e].sort((r,s)=>{let i=r.last_active_at||r.created_at||"";return(s.last_active_at||s.created_at||"").localeCompare(i)}).slice(0,8);return n.length?l`
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
                <td className="py-3 pr-4"><${q} tone=${Ni(r.role)} label=${r.role||"member"} /></td>
                <td className="py-3 pr-4"><${q} tone=${Si(r.status)} label=${r.status||"active"} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${r.job_count??0}</td>
                <td className="py-3 text-xs text-iron-300">${dr(r.last_active_at)}</td>
              </tr>
            `)}
        </tbody>
      </table>
    </div>
  `:l`<p className="py-4 text-sm text-iron-300">${a("admin.dashboard.noUsers")}</p>`}function Jk({onSelectUser:e,onNavigateTab:t}){let a=R(),n=qk(),{users:r,query:s}=$i(),i=n.data||{},o=Hk(r),u=i.usage_30d||{},c=i.jobs||{};return n.isLoading||s.isLoading?l`
      <div className="space-y-5">
        <${I} className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            ${[1,2,3,4].map(f=>l`<div key=${f} className="v2-skeleton h-28 rounded-lg" />`)}
          </div>
        <//>
      </div>
    `:l`
    <div className="space-y-5">
      <${I} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.dashboard.systemOverview")}</h3>
          ${i.uptime_seconds!=null&&l`
            <span className="font-mono text-xs text-iron-300">${a("admin.dashboard.uptime",{value:Kk(i.uptime_seconds)})}</span>
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
        <${HM} users=${r} onSelectUser=${e} />
      <//>
    </div>
  `}var QM=[{value:"day",label:"24h"},{value:"week",label:"7d"},{value:"month",label:"30d"}];function VM({value:e,max:t}){let a=t>0?e/t*100:0;return l`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{width:`${Math.max(a,1)}%`}}
      />
    </div>
  `}function Xk({onSelectUser:e}){let t=R(),[a,n]=p.default.useState("day"),r=gd(a),s=r.data?.usage||[],i=Vk(s),o=Gk(s),u=Yk(i),c=i.length>0?i[0].cost:0;return r.isLoading?l`
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
            ${QM.map(d=>l`
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
                <${et} label=${t("admin.usage.inputTokens")} value=${Xa(u.input_tokens)} tone="muted" />
                <${et} label=${t("admin.usage.outputTokens")} value=${Xa(u.output_tokens)} tone="muted" />
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
                          ${wi(d.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${d.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Xa(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Xa(d.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${VM} value=${d.cost} max=${c} />
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
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Xa(d.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Xa(d.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${Aa(d.cost.toFixed(2))}</td>
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
  `}function Zk({userId:e,onBack:t}){let a=R(),n=Ik(e),r=gd("month",e),{suspendUser:s,activateUser:i,updateUser:o,deleteUser:u,createToken:c,newToken:d,clearToken:f}=$i(),[m,h]=p.default.useState(null),[b,y]=p.default.useState(!1),w=n.data,g=r.data?.usage||[];if(p.default.useEffect(()=>{w&&m===null&&h(w.role)},[w]),n.isLoading)return l`
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
    `;if(!w)return null;let v=async()=>{m&&m!==w.role&&await o(w.id,{role:m})},x=async()=>{await u(w.id),t()},$=async()=>{let S=window.prompt(a("admin.users.tokenNamePrompt",{name:w.display_name||a("admin.users.userFallback")}));S&&await c(w.id,S)};return l`
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
            <h2 className="text-2xl font-semibold tracking-tight text-white">${w.display_name||w.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <${q} tone=${Ni(w.role)} label=${w.role||"member"} />
              <${q} tone=${Si(w.status)} label=${w.status||"active"} />
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
            <button onClick=${f} className="text-iron-300 hover:text-white">
              <${D} name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      `}

      <div className="grid gap-5 lg:grid-cols-2">
        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.profile")}</h3>
          <${mr} label=${a("admin.user.id")}>
            <span className="font-mono text-xs">${w.id}</span>
          <//>
          <${mr} label=${a("admin.user.email")}>${w.email||a("admin.user.notSet")}<//>
          <${mr} label=${a("admin.user.created")}>${dr(w.created_at)}<//>
          <${mr} label=${a("admin.user.lastLogin")}>${dr(w.last_login_at)}<//>
          ${w.created_by&&l`
            <${mr} label=${a("admin.user.createdBy")}>
              <span className="font-mono text-xs">${wi(w.created_by)}</span>
            <//>
          `}
        <//>

        <${I} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.summary")}</h3>
          <${mr} label=${a("admin.user.jobs")}>${w.job_count??0}<//>
          <${mr} label=${a("admin.user.totalCost")}>${Aa(w.total_cost)}<//>
          <${mr} label=${a("admin.user.lastActive")}>${dr(w.last_active_at)}<//>
        <//>
      </div>

      <${I} className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${a("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">${a("admin.user.currentRole")}</label>
            <select
              value=${m||w.role}
              onChange=${S=>h(S.target.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">${a("admin.users.member")}</option>
              <option value="admin">${a("admin.users.admin")}</option>
            </select>
          </div>
          <${A} onClick=${v} disabled=${!m||m===w.role}>
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
                    ${g.map((S,C)=>l`
                        <tr key=${C} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">${S.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">${(S.call_count||0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Xa(S.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${Xa(S.output_tokens)}</td>
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
              ${a("admin.users.deleteUserDesc",{name:w.display_name})}
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
  `}function GM(e){return[{value:"all",label:e("admin.users.filter.all")},{value:"active",label:e("admin.users.filter.active")},{value:"suspended",label:e("admin.users.filter.suspended")},{value:"admin",label:e("admin.users.filter.admins")}]}function YM({token:e,onDismiss:t}){let a=R(),[n,r]=p.default.useState(!1),s=()=>{navigator.clipboard&&(navigator.clipboard.writeText(e),r(!0),setTimeout(()=>r(!1),2e3))};return l`
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
          <${D} name="close" className="h-4 w-4" />
        </button>
      </div>
    </div>
  `}function JM({onCreate:e,isCreating:t,error:a}){let n=R(),[r,s]=p.default.useState(""),[i,o]=p.default.useState(""),[u,c]=p.default.useState("member"),[d,f]=p.default.useState(!1),m=async h=>{h.preventDefault(),r.trim()&&(await e({display_name:r.trim(),email:i.trim()||void 0,role:u}),s(""),o(""),f(!1))};return d?l`
    <${I} className="p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${n("admin.users.createUser")}</h3>
      <form onSubmit=${m} className="space-y-4">
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
          <${A} variant="ghost" type="button" onClick=${()=>f(!1)}>${n("admin.users.cancel")}<//>
        </div>
      </form>
    <//>
  `:l`
      <${A} variant="secondary" onClick=${()=>f(!0)}>
        <${D} name="plus" className="mr-2 h-4 w-4" />
        ${n("admin.users.newUser")}
      <//>
    `}function XM({title:e,message:t,confirmLabel:a,onConfirm:n,onCancel:r}){let s=R();return l`
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
  `}function ZM({user:e,onSelect:t,onSuspend:a,onActivate:n,onChangeRole:r,onCreateToken:s}){let i=R();return l`
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick=${()=>t(e.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            ${e.display_name||e.id}
          </button>
          <${q} tone=${Ni(e.role)} label=${e.role||"member"} />
          <${q} tone=${Si(e.status)} label=${e.status||"active"} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          ${e.email&&l`<span className="font-mono text-xs text-iron-300">${e.email}</span>`}
          <span className="font-mono text-xs text-iron-700">${wi(e.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          ${e.job_count!=null?i("admin.users.jobsCount",{count:e.job_count}):""}
          ${e.total_cost!=null?` \xB7 ${Aa(e.total_cost)}`:""}
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
  `}function Wk({selectedUserId:e,onSelectUser:t}){let a=R(),{users:n,query:r,isForbidden:s,createUser:i,isCreating:o,createError:u,updateUser:c,deleteUser:d,suspendUser:f,activateUser:m,createToken:h,newToken:b,clearToken:y}=$i(),[w,g]=p.default.useState(""),[v,x]=p.default.useState("all"),[$,S]=p.default.useState(null),C=Qk(n,{search:w,filter:v}),_=GM(a),T=O=>{S({title:a("admin.users.suspendTitle"),message:a("admin.users.suspendDesc"),confirmLabel:a("admin.users.suspend"),onConfirm:()=>{f(O),S(null)}})},M=async(O,U)=>{let k=window.prompt(a("admin.users.tokenNamePrompt",{name:U||a("admin.users.userFallback")}));k&&await h(O,k)};return r.isLoading?l`
      <${I} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        ${[1,2,3].map(O=>l`
          <div key=${O} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        `)}
      <//>
    `:s?l`
      <${I} className="p-6 sm:p-8">
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
        <${YM}
          token=${b.token||b.plaintext_token}
          onDismiss=${y}
        />
      `}

      <${JM} onCreate=${i} isCreating=${o} error=${u} />

      <${I} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${a("admin.users.title",{count:C.length,total:n.length})}
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

        ${C.length===0?l`<p className="py-4 text-sm text-iron-300">${a("admin.users.noMatch")}</p>`:C.map(O=>l`
                <${ZM}
                  key=${O.id}
                  user=${O}
                  onSelect=${t}
                  onSuspend=${T}
                  onActivate=${m}
                  onChangeRole=${(U,k)=>c(U,{role:k})}
                  onCreateToken=${M}
                />
              `)}
      <//>

      ${$&&l`
        <${XM}
          title=${$.title}
          message=${$.message}
          confirmLabel=${$.confirmLabel}
          onConfirm=${$.onConfirm}
          onCancel=${()=>S(null)}
        />
      `}
    </div>
  `}function eR(){let{tab:e="dashboard"}=it(),t=fe(),[a,n]=p.default.useState(null),r=p.default.useCallback(o=>{n(o),t("/admin/users")},[t]),s=p.default.useCallback(()=>{n(null)},[]),i={dashboard:l`<${Jk}
      onSelectUser=${r}
      onNavigateTab=${o=>t("/admin/"+o)}
    />`,users:a?l`<${Zk} userId=${a} onBack=${s} />`:l`<${Wk}
          selectedUserId=${a}
          onSelectUser=${r}
        />`,usage:l`<${Xk} onSelectUser=${r} />`};return i[e]?l`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${i[e]}</div>
      </div>
    </div>
  `:l`<${ot} to="/admin/dashboard" replace />`}var WM=2e3,eO=500,tO=2e3,aO=new Set([403,404]),nO=[["threadId","thread_id","logs.scope.thread"],["runId","run_id","logs.scope.run"],["turnId","turn_id","logs.scope.turn"],["toolCallId","tool_call_id","logs.scope.toolCall"],["toolName","tool_name","logs.scope.tool"],["source","source","logs.scope.source"]];function rO(e=globalThis.location,t=null){let a=new URLSearchParams(e?.search||""),n={active:[]};for(let[r,s,i]of nO){let o=a.get(s)?.trim();o?(n[r]=o,n.active.push({key:r,param:s,labelKey:i,value:o})):n[r]=null}return!n.threadId&&t&&(n.threadId=t),n}function tR({isAdmin:e=!1,defaultThreadId:t=null}={}){let a=Pe(),n=a?.search||"",r=p.default.useMemo(()=>rO(a,t),[t,n]),{runId:s,source:i,threadId:o,toolCallId:u,toolName:c,turnId:d}=r,[f,m]=p.default.useState([]),[h,b]=p.default.useState("all"),[y,w]=p.default.useState(""),[g,v]=p.default.useState(!1),[x,$]=p.default.useState(!0),[S,C]=p.default.useState(!0),[_,T]=p.default.useState(null),M=p.default.useRef(new Set),O=p.default.useRef(0),U=!e&&!o;p.default.useEffect(()=>{O.current+=1,m([]),T(null)},[e,s,i,o,u,c,d]);let k=p.default.useCallback(async()=>{if(U){C(!1);return}let re=++O.current;C(!0);try{let me={limit:eO,level:h==="all"?null:h,target:y.trim()||null,threadId:o,runId:s,turnId:d,toolCallId:u,toolName:c,source:i},pe;try{pe=await(e?zx(me):Ap(me))}catch(pt){if(!e||!aO.has(pt?.status))throw pt;pe=await Ap(me)}if(re!==O.current)return;let Ee=M.current,bt=B2(pe).entries.filter(pt=>!Ee.has(pt.id));m(bt),T(null)}catch(me){if(re!==O.current)return;T(me)}finally{re===O.current&&C(!1)}},[e,h,U,s,i,y,o,u,c,d]);p.default.useEffect(()=>{k()},[k]),p.default.useEffect(()=>{if(g||U)return;let re=setInterval(k,WM);return()=>clearInterval(re)},[k,U,g]);let z=p.default.useCallback(()=>{v(re=>!re)},[]),Z=p.default.useCallback(()=>{let re=[...M.current,...f.map(me=>me.id)].slice(-tO);M.current=new Set(re),m([])},[f]);return{entries:f,totalCount:f.length,paused:g,togglePause:z,clearEntries:Z,levelFilter:h,setLevelFilter:b,targetFilter:y,setTargetFilter:w,autoScroll:x,setAutoScroll:$,serverLevel:null,changeServerLevel:async()=>{},scope:r,needsThreadScope:U,status:U?"needs_scope":_?"error":S?"loading":"ready",isLoading:S,error:_}}var sO=["all","trace","debug","info","warn","error"],iO=["trace","debug","info","warn","error"],aR={trace:"text-[var(--v2-text-muted)]",debug:"text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",info:"text-[var(--v2-text-strong)]",warn:"text-yellow-400",error:"text-red-400"},oO={warn:"bg-yellow-500/5",error:"bg-red-500/8"};function lO({entry:e}){let t=R(),[a,n]=p.default.useState(!1),r=e.timestamp?e.timestamp.substring(11,23):"",s=aR[e.level]||aR.info,i=oO[e.level]||"",o=[{key:"thread_id",labelKey:"logs.scope.thread",value:e.threadId},{key:"run_id",labelKey:"logs.scope.run",value:e.runId},{key:"turn_id",labelKey:"logs.scope.turn",value:e.turnId},{key:"tool_call_id",labelKey:"logs.scope.toolCall",value:e.toolCallId},{key:"tool_name",labelKey:"logs.scope.tool",value:e.toolName},{key:"source",labelKey:"logs.scope.source",value:e.source}].filter(u=>!!u.value);return l`
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
  `}function nR({value:e,onChange:t,options:a,labelKey:n,t:r}){return l`
    <select
      value=${e}
      onChange=${s=>t(s.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${a.map(s=>l`<option key=${s} value=${s}>${r(n(s))}</option>`)}
    </select>
  `}function uO({label:e,value:t,scopeKey:a}){return l`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${a}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${e}: ${t}`}
    >
      <span className="uppercase tracking-[0.08em]">${e}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${t}</span>
    </span>
  `}function rR(){let e=R(),{isAdmin:t=!1,threadsState:a}=wa()||{},{entries:n,totalCount:r,paused:s,togglePause:i,clearEntries:o,levelFilter:u,setLevelFilter:c,targetFilter:d,setTargetFilter:f,autoScroll:m,setAutoScroll:h,serverLevel:b,changeServerLevel:y,scope:w,isLoading:g,error:v,needsThreadScope:x}=tR({isAdmin:t,defaultThreadId:t?null:a?.activeThreadId||null}),$=p.default.useRef(null),S=p.default.useRef(!0);p.default.useEffect(()=>{m&&S.current&&$.current&&($.current.scrollTop=0)},[n,m]);let C=p.default.useCallback(M=>{S.current=M.currentTarget.scrollTop<=48},[]),_=n.length>0,T=w?.active||[];return l`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${nR}
          value=${u}
          onChange=${c}
          options=${sO}
          labelKey=${M=>M==="all"?"logs.levelAll":`logs.level.${M}`}
          t=${e}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${d}
          onInput=${M=>f(M.target.value)}
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
              checked=${m}
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

        ${T.length>0&&l`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${e("logs.scoped")}</span>
            ${T.map(M=>l`<${uO} key=${M.param} scopeKey=${M.param} label=${e(M.labelKey)} value=${M.value} />`)}
            <a
              href="/v2/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${e("logs.clearScope")}
            </a>
          </div>
        `}

        <!-- Server log level -->
        ${b!=null&&l`
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>${e("logs.serverLevel")}</span>
            <${nR}
              value=${b}
              onChange=${y}
              options=${iO}
              labelKey=${M=>`logs.level.${M}`}
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
        onScroll=${C}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${v&&_?l`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${e("error.loadFailed",{what:e("nav.logs"),message:v.message||v.statusText||"Request failed"})}
              </div>
            `:null}
        ${x?l`
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
              `:_?n.map(M=>l`<${lO} key=${M.id} entry=${M} />`):l`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${e("logs.empty")}
              </div>
            `}
      </div>
    </div>
  `}function iR(){return l`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `}function cO({auth:e}){let t=fe(),n=Pe().state?.from,r=n?`${n.pathname||Br}${n.search||""}${n.hash||""}`:Br,s=`/v2${r==="/"?"":r}`,i=p.default.useCallback(o=>{e.signIn(o),t(r,{replace:!0})},[e,r,t]);return e.isChecking?l`<${iR} />`:e.isAuthenticated?l`<${ot} to=${r} replace />`:l`<${y1}
    initialToken=${e.token}
    error=${e.error}
    oauthRedirectAfter=${s}
    onSubmit=${i}
  />`}function dO({auth:e,children:t}){let a=Pe();return e.isChecking?l`<${iR} />`:e.isAuthenticated?t:l`<${ot} to="/login" replace state=${{from:a}} />`}function mO({auth:e}){return l`
    <${dO} auth=${e}>
      <${Vw}
        token=${e.token}
        profile=${e.profile}
        isChecking=${e.isChecking}
        isAdmin=${e.isAdmin}
        rebornProjectsEnabled=${e.rebornProjectsEnabled}
        onSignOut=${e.signOut}
      />
    <//>
  `}function sR({auth:e}){return e.isAdmin?l`<${eR} />`:l`<${ot} to=${Br} replace />`}function oR(){let e=k$();return l`
    <${Rp} basename="/v2">
      <${Np}>
        <${be} path="/login" element=${l`<${cO} auth=${e} />`} />
        <${be} path="/" element=${l`<${mO} auth=${e} />`}>
          <${be} index element=${l`<${ot} to=${Br} replace />`} />
          <${be} path="overview" element=${l`<${ot} to=${Br} replace />`} />
          <${be} path="welcome" element=${l`<${V2} />`} />
          <${be} path="chat" element=${l`<${bh} />`} />
          <${be} path="chat/:threadId" element=${l`<${bh} />`} />
          <${be} path="workspace" element=${l`<${$h} />`} />
          <${be} path="workspace/*" element=${l`<${$h} />`} />
          <${be} path="projects" element=${l`<${il} />`} />
          <${be} path="projects/:projectId" element=${l`<${il} />`} />
          <${be} path="projects/:projectId/missions/:missionId" element=${l`<${il} />`} />
          <${be} path="projects/:projectId/threads/:threadId" element=${l`<${il} />`} />
          <${be} path="missions" element=${l`<${Sh} />`} />
          <${be} path="missions/:missionId" element=${l`<${Sh} />`} />
          <${be} path="jobs" element=${l`<${kh} />`} />
          <${be} path="jobs/:jobId" element=${l`<${kh} />`} />
          <${be} path="routines" element=${l`<${Ch} />`} />
          <${be} path="routines/:routineId" element=${l`<${Ch} />`} />
          <${be} path="automations" element=${l`<${WN} />`} />
          <${be} path="extensions" element=${l`<${Bh} />`} />
          <${be} path="extensions/:tab" element=${l`<${Bh} />`} />
          <${be} path="logs" element=${l`<${rR} />`} />
          <${be} path="settings" element=${l`<${Kh} />`} />
          <${be} path="settings/:tab" element=${l`<${Kh} />`} />
          <${be} path="admin" element=${l`<${sR} auth=${e} />`} />
          <${be} path="admin/:tab" element=${l`<${sR} auth=${e} />`} />
        <//>
        <${be} path="*" element=${l`<${ot} to=${Br} replace />`} />
      <//>
    <//>
  `}Qh("en",{"language.name":"English","language.switch":"Language changed","common.unknown":"Unknown","common.cancel":"Cancel","common.delete":"Delete","common.edit":"Edit","common.loading":"Loading...","common.save":"Save","common.saving":"Saving...","common.done":"Done","common.send":"Send","nav.chat":"Chat","nav.close":"Close","nav.workspace":"Workspace","nav.projects":"Projects","nav.jobs":"Jobs","nav.routines":"Routines","nav.automations":"Automations","nav.missions":"Missions","nav.extensions":"Extensions","nav.settings":"Settings","nav.admin":"Admin","nav.logs":"Logs","nav.docs":"Documentation","nav.sectionWork":"Work","nav.sectionSystem":"System","theme.switchToLight":"Switch to light theme","theme.switchToDark":"Switch to dark theme","theme.light":"Light theme","theme.dark":"Dark theme","header.signOut":"Sign out","status.online":"online","status.offline":"offline","status.checking":"checking","login.tagline":"Gateway v2","login.hero":"Local agent control without losing the operator trail.","login.heroSub":"Token access keeps the browser console tied to the same gateway runtime, approvals, tools, and thread state.","login.bearerAuth":"Bearer auth","login.bearerDesc":"Paste the local gateway token to open the operator surface.","login.console":"IronClaw console","login.secureSub":"Secure access to the local agent gateway.","login.tokenLabel":"Gateway token","login.tokenRequired":"Gateway token is required","login.tokenPlaceholder":"Paste your auth token","login.tokenHint":"Use the token printed by the local gateway process.","login.connect":"Connect","login.oauthDivider":"or continue with","login.oauthProvider":"Continue with {provider}","chat.heroTitle":"Hello, what do you need help with?","chat.heroDesc":"Start with a goal, a repo question, a review request, or work you want inspected.","chat.emptyTitle":"Start with a concrete operator task.","chat.emptyDesc":"Send a message or ask for a gateway check. The workspace keeps approvals and runtime activity visible as the turn progresses.","chat.suggestion1":"Map the current gateway state","chat.suggestion1Desc":"Inspect runtime health, channels, tools, and open work.","chat.suggestion2":"Review recent thread activity","chat.suggestion2Desc":"Look for correctness risks, blocked approvals, and follow-ups.","chat.suggestion3":"Draft an extension readiness check","chat.suggestion3Desc":"Verify setup, auth, pairing, and available capabilities.","chat.placeholder":"Message IronClaw...","chat.heroPlaceholder":"Ask IronClaw anything.","chat.followUpPlaceholder":"Ask for follow-up changes","chat.send":"Send message","chat.attachFiles":"Attach files","chat.attachmentRemove":"Remove attachment","chat.attachmentDropHint":"Drop files to attach","chat.attachmentTooMany":"You can attach at most {max} files per message.","chat.attachmentTooLarge":"{name} is too large (max {max} per file).","chat.attachmentTotalTooLarge":"Attachments exceed the {max} total limit.","chat.attachmentUnsupportedType":"{name} is not a supported file type.","chat.attachmentReadFailed":"Could not read {name}.","chat.attachmentStagingFailed":"Could not attach the selected files.","chat.fileDownloadFailed":"Couldn't download that file.","chat.modeAutoReview":"Auto-review","chat.runtimeLocal":"Work locally","chat.statusWorking":"Working","chat.identityUser":"You","chat.identityAssistant":"IronClaw","chat.jumpToLatest":"Jump to latest","shortcuts.title":"Keyboard shortcuts","shortcuts.send":"Send message","shortcuts.newline":"New line","shortcuts.help":"Show this help","shortcuts.close":"Close","chat.conversations":"Conversations","chat.threads":"{count} threads","chat.newThread":"New","chat.creating":"Creating","chat.selectConversation":"Select conversation","chat.noConversations":"No conversations yet. Start a thread from the composer suggestions.","chat.turns":"{count} turns","connection.connected":"Connected","connection.reconnecting":"Reconnecting...","connection.disconnected":"Disconnected","connection.connecting":"Connecting...","connection.paused":"Paused while tab is hidden","approval.title":"Approval required","approval.approve":"Approve","approval.deny":"Deny","approval.always":"Always","approval.approveAndAlways":"Approve & always allow","approval.alwaysAllowToolLabel":"Always allow {tool} without asking","approval.thisTool":"this tool","approval.viewFullCommand":"View full command","approval.showCommandPreview":"Show preview","tool.tabDetails":"Details","tool.tabParameters":"Parameters","tool.tabResult":"Result","tool.tabError":"Error","tool.tabDeclined":"Declined","tool.noDetail":"No additional detail.","tool.runFile":"explored {n} file","tool.runFiles":"explored {n} files","tool.runSearch":"{n} search","tool.runSearches":"{n} searches","tool.runCommand":"ran {n} command","tool.runCommands":"ran {n} commands","tool.runOther":"{n} tool","tool.runOthers":"{n} tools","tool.exitOk":"succeeded","tool.exitError":"failed","tool.exitDeclined":"declined","tool.exitRunning":"running\u2026","tool.riskRead":"reads","tool.riskWrite":"writes files","tool.riskExec":"runs commands","tool.riskNetwork":"network","authGate.title":"Authentication required","authGate.tokenLabel":"Access token","authGate.tokenPlaceholder":"Paste access token","authGate.tokenRequired":"A token is required.","authGate.submit":"Use token","authGate.submitting":"Checking...","authGate.cancel":"Cancel","authGate.oauthTitle":"Authorization required","authGate.oauthAccountLabel":"Account:","authGate.openAuthorization":"Open {provider} authorization","authGate.reopenAuthorization":"Re-open {provider} authorization","authGate.oauthWaiting":"Waiting for authorization to complete\u2026 You can close the popup tab once you\u2019ve approved access.","authGate.expiresAt":"Expires","authGate.oauthProviderFallback":"the provider","authGate.serviceUnavailable":"Service unavailable","authGate.pillAuthorize":"Authorize","authGate.pillEnterToken":"Enter token","authGate.unsupportedChallenge":"Open settings to complete this authentication step.","authGate.submitFailed":"Could not save the token.","authGate.resolveFailedAfterTokenSaved":"Token saved. Could not resume the blocked run; retry to resume it.","error.gatewayConnection":"Unable to connect to the gateway","error.saveFailed":"Save failed: {message}","error.loadFailed":"Failed to load {what}: {message}","extensions.installed":"Installed","extensions.channels":"Channels","extensions.mcp":"MCP Servers","extensions.registry":"Registry","settings.inference":"Inference","settings.agent":"Agent","settings.channels":"Channels","settings.networking":"Networking","settings.tools":"Tools","settings.skills":"Skills","settings.traceCommons":"Trace Commons","settings.users":"Users","settings.language":"Language","traceCommons.title":"Trace Commons credits","traceCommons.description":"Credit earned for contributed redacted traces, scoped to your account.","traceCommons.emptyState":"Not enrolled \u2014 ask your agent to onboard with a Trace Commons invite.","traceCommons.loadFailed":"Could not load Trace Commons credits.","traceCommons.enrollment":"Enrollment","traceCommons.enrolled":"Enrolled","traceCommons.notEnrolled":"Not enrolled","traceCommons.pendingCredit":"Pending credit","traceCommons.pendingCreditDesc":"Earned but not yet finalized","traceCommons.finalCredit":"Final credit","traceCommons.finalCreditDesc":"Confirmed credit","traceCommons.delayedLedger":"Delayed ledger","traceCommons.delayedLedgerDesc":"Can still change after review","traceCommons.submissions":"Submissions","traceCommons.submissionsValue":"{submitted} submitted, {accepted} accepted of {total} total","traceCommons.cardAccepted":"Accepted {accepted} / {submitted}","traceCommons.cardHeld":"{count} held for review","traceCommons.heldTitle":"Held for review","traceCommons.heldDescription":"Held because of higher privacy risk; review and authorize to submit.","traceCommons.authorize":"Authorize","traceCommons.authorizing":"Authorizing\u2026","traceCommons.lastSubmission":"Last submission","traceCommons.lastSync":"Last credit sync","traceCommons.lastSyncDesc":"Local view as of last sync","traceCommons.never":"never","traceCommons.recentExplanations":"Recent credit explanations","traceCommons.note":"Local view as of last sync \u2014 the authoritative credit ledger is server-side. Final credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.","settings.back":"Back","settings.searchPlaceholder":"Search settings...","settings.clearSearch":"Clear search","settings.noMatchingSettings":'No settings match "{query}"',"settings.manageJson":"Settings JSON","settings.export":"Export","settings.import":"Import","settings.importing":"Importing...","settings.exportSuccess":"Settings exported","settings.importSuccess":"Settings imported","settings.importInvalid":"Selected file must contain a settings object","settings.importFailed":"Import failed: {message}","settings.restartRequired":"Some changes require a restart to take effect.","settings.restartNow":"Restart now","settings.restartStarting":"Restarting...","settings.restartUnavailable":"Restart from the web UI isn't available yet. Restart the gateway process manually to apply pending changes.","restart.title":"Restart IronClaw","restart.description":"Restart the gateway process to apply pending changes.","restart.warning":"Running tasks may be interrupted while the gateway restarts.","restart.cancel":"Cancel","restart.confirm":"Confirm restart","restart.progressTitle":"Restarting IronClaw","tee.title":"TEE Attestation","tee.verified":"Verified runtime attestation available","tee.imageDigest":"Image digest","tee.tlsFingerprint":"TLS certificate fingerprint","tee.reportData":"Report data","tee.vmConfig":"VM config","tee.loading":"Loading attestation report...","tee.loadFailed":"Could not load attestation report","tee.copyReport":"Copy report","tee.copied":"Copied","llm.active":"Active","llm.addProvider":"Add provider","llm.adapter":"Adapter","llm.apiKey":"API key","llm.apiKeyPlaceholder":"Leave blank to keep the stored key","llm.baseUrl":"Base URL","llm.baseUrlRequired":"Base URL is required.","llm.builtin":"Built-in","llm.configure":"Configure","llm.configureProvider":"Configure {name}","llm.configureToUse":"Configure this provider before activating it.","llm.confirmDelete":'Delete provider "{id}"?',"llm.defaultModel":"Default model","llm.editProvider":"Edit provider","llm.fetchModels":"Fetch models","llm.fetchingModels":"Fetching...","llm.fieldsRequired":"Display name and provider ID are required.","llm.idTaken":'Provider ID "{id}" is already used.',"llm.invalidId":"Use lowercase letters, numbers, hyphens, or underscores.","llm.model":"Model","llm.modelRequired":"A model is required.","llm.modelsFetched":"{count} models found.","llm.modelsFetchFailed":"No models were returned.","llm.newProvider":"New provider","llm.none":"None","llm.notConfigured":"Not configured","llm.providerActivated":"Switched to {name}.","llm.providerAdded":'Added provider "{name}".',"llm.providerConfigured":"Configured {name}.","llm.providerDeleted":"Provider deleted.","llm.providerId":"Provider ID","llm.providerName":"Display name","llm.providerUpdated":'Updated provider "{name}".',"llm.providers":"LLM providers","llm.providersDesc":"Manage built-in and custom inference providers.","onboarding.title":"Welcome to IronClaw","onboarding.subtitle":"Choose an AI provider to get started. You can change or add more later in Settings.","onboarding.setUp":"Set up","onboarding.signIn":"Sign in","onboarding.nearWallet":"NEAR Wallet","onboarding.ready":"Ready","onboarding.moreInSettings":"Need a different provider? Configure any of them in","onboarding.providerNearai":"NEAR AI","onboarding.providerNearaiDesc":"Free hosted models. Use an API key or SSO.","onboarding.providerCodex":"ChatGPT subscription","onboarding.providerCodexDesc":"Use your existing ChatGPT Plus or Pro plan.","onboarding.providerOpenai":"OpenAI API","onboarding.providerOpenaiDesc":"Bring your own OpenAI API key.","onboarding.providerAnthropic":"Anthropic API","onboarding.providerAnthropicDesc":"Bring your own Anthropic API key.","onboarding.providerOllama":"Local Ollama","onboarding.providerOllamaDesc":"Run open models locally. No API key needed.","onboarding.nearaiWaiting":"Waiting for NEAR AI sign-in in the opened tab\u2026","onboarding.nearaiTimeout":"NEAR AI sign-in timed out. Please try again.","onboarding.nearaiFailed":"NEAR AI sign-in failed. Please try again.","onboarding.nearaiLocalSso":"NEAR AI browser sign-in (GitHub, Google, NEAR Wallet) isn't supported on localhost \u2014 NEAR AI rejects local callback URLs. Add a NEAR AI API key instead, or run behind a public URL.","onboarding.codexSignIn":"Sign in with ChatGPT","onboarding.codexEnterCode":"Enter this code in the opened tab to authorize:","onboarding.codexWaiting":"Waiting for ChatGPT authorization in the opened tab\u2026","onboarding.codexTimeout":"ChatGPT sign-in timed out. Please try again.","onboarding.codexFailed":"ChatGPT sign-in failed. Please try again.","llm.testConnection":"Test connection","llm.testing":"Testing...","llm.use":"Use","llm.groupActive":"Active","llm.groupReady":"Ready to use","llm.groupSetup":"Needs setup","llm.expandDetails":"Show details","llm.collapseDetails":"Hide details","llm.missingApiKey":"Missing API key","llm.missingBaseUrl":"Missing base URL","llm.addApiKey":"Add API key","settings.group.embeddings":"Embeddings","settings.group.sampling":"Sampling","settings.field.embeddingsEnabled":"Enable embeddings","settings.field.embeddingsEnabledDesc":"Semantic search over workspace memory","settings.field.embeddingsProvider":"Provider","settings.field.embeddingsProviderDesc":"Embedding model provider","settings.field.embeddingsModel":"Model","settings.field.embeddingsModelDesc":"Embedding model identifier","settings.field.temperature":"Temperature","settings.field.temperatureDesc":"Default sampling temperature (0.0\u20132.0)","settings.group.core":"Core","settings.group.heartbeat":"Heartbeat","settings.group.sandbox":"Sandbox","settings.group.routines":"Routines","settings.group.safety":"Safety","settings.group.skills":"Skills","settings.group.search":"Search","settings.field.agentName":"Agent name","settings.field.agentNameDesc":"Display name for the assistant","settings.field.maxParallelJobs":"Max parallel jobs","settings.field.maxParallelJobsDesc":"Concurrent background job limit","settings.field.jobTimeout":"Job timeout","settings.field.jobTimeoutDesc":"Seconds before a job is marked stuck","settings.field.maxToolIterations":"Max tool iterations","settings.field.maxToolIterationsDesc":"Tool call limit per turn","settings.field.planning":"Planning","settings.field.planningDesc":"Enable multi-step planning before execution","settings.field.autoApproveEligibleTools":"Always allow eligible tools","settings.field.autoApproveEligibleToolsDesc":"Applies to tools set to \u201CFollow global\u201D. Per-tool settings override this; hard-floor approval gates still ask.","settings.field.timezone":"Timezone","settings.field.timezoneDesc":"IANA timezone for scheduled work","settings.field.sessionIdleTimeout":"Session idle timeout","settings.field.sessionIdleTimeoutDesc":"Seconds of inactivity before session ends","settings.field.stuckThreshold":"Stuck threshold","settings.field.stuckThresholdDesc":"Seconds before a job is considered stuck","settings.field.maxRepairAttempts":"Max repair attempts","settings.field.maxRepairAttemptsDesc":"Retry limit for stuck job recovery","settings.field.dailyCostLimit":"Daily cost limit (cents)","settings.field.dailyCostLimitDesc":"Maximum spend per day in cents","settings.field.actionsPerHour":"Actions per hour limit","settings.field.actionsPerHourDesc":"Hourly action rate cap","settings.field.allowLocalTools":"Allow local tools","settings.field.allowLocalToolsDesc":"Enable filesystem and shell access","settings.field.heartbeatEnabled":"Enable heartbeat","settings.field.heartbeatEnabledDesc":"Periodic proactive execution","settings.field.heartbeatInterval":"Interval","settings.field.heartbeatIntervalDesc":"Seconds between heartbeat runs","settings.field.heartbeatNotifyChannel":"Notify channel","settings.field.heartbeatNotifyChannelDesc":"Channel to send heartbeat notifications","settings.field.heartbeatNotifyUser":"Notify user","settings.field.heartbeatNotifyUserDesc":"User ID to notify on findings","settings.field.quietHoursStart":"Quiet hours start","settings.field.quietHoursStartDesc":"Hour (0\u201323) to begin suppression","settings.field.quietHoursEnd":"Quiet hours end","settings.field.quietHoursEndDesc":"Hour (0\u201323) to end suppression","settings.field.heartbeatTimezone":"Timezone","settings.field.heartbeatTimezoneDesc":"IANA timezone for quiet hours","settings.field.sandboxEnabled":"Enable sandbox","settings.field.sandboxEnabledDesc":"Docker-based tool execution","settings.field.sandboxPolicy":"Policy","settings.field.sandboxPolicyDesc":"Container filesystem access level","settings.field.sandboxTimeout":"Timeout","settings.field.sandboxTimeoutDesc":"Container execution time limit","settings.field.sandboxMemoryLimit":"Memory limit (MB)","settings.field.sandboxMemoryLimitDesc":"Container memory ceiling","settings.field.sandboxImage":"Docker image","settings.field.sandboxImageDesc":"Container image for sandbox runs","settings.field.routinesMaxConcurrent":"Max concurrent","settings.field.routinesMaxConcurrentDesc":"Parallel routine execution limit","settings.field.routinesDefaultCooldown":"Default cooldown","settings.field.routinesDefaultCooldownDesc":"Seconds between routine runs","settings.field.safetyMaxOutput":"Max output length","settings.field.safetyMaxOutputDesc":"Character limit on tool output","settings.field.safetyInjectionCheck":"Injection detection","settings.field.safetyInjectionCheckDesc":"Scan tool outputs for prompt injection","settings.field.skillsMaxActive":"Max active skills","settings.field.skillsMaxActiveDesc":"Concurrent skill attachment limit","settings.field.skillsMaxContextTokens":"Max context tokens","settings.field.skillsMaxContextTokensDesc":"Token budget for injected skill prompts","settings.field.fusionStrategy":"Fusion strategy","settings.field.fusionStrategyDesc":"Result merging method for hybrid search","settings.group.gateway":"Gateway","settings.group.tunnel":"Tunnel","settings.field.gatewayHost":"Host","settings.field.gatewayHostDesc":"Gateway bind address","settings.field.gatewayPort":"Port","settings.field.gatewayPortDesc":"Gateway listen port","settings.field.tunnelProvider":"Provider","settings.field.tunnelProviderDesc":"Public tunnel service","settings.field.tunnelPublicUrl":"Public URL","settings.field.tunnelPublicUrlDesc":"Static tunnel endpoint","channels.builtIn":"Built-in channels","channels.messaging":"Messaging channels","channels.availableChannels":"Available channels","channels.mcpServers":"MCP servers","channels.webGateway":"Web Gateway","channels.webGatewayDesc":"Browser-based chat with SSE streaming","channels.httpWebhook":"HTTP Webhook","channels.httpWebhookDesc":"Inbound webhook endpoint for external integrations","channels.cli":"CLI","channels.cliDesc":"Terminal interface with TUI or simple REPL","channels.repl":"REPL","channels.replDesc":"Minimal read-eval-print loop for testing","channels.slack":"Slack","channels.slackDesc":"Tenant app channel for DMs and app mentions","channels.slackDetail":"Tenant Slack app install","channels.statusOn":"on","channels.statusOff":"off","channels.ready":"ready","channels.authNeeded":"auth needed","channels.pairing":"pairing","channels.setup":"setup","channels.active":"active","channels.inactive":"inactive","channels.available":"available","channels.slackAccessTitle":"Slack team agents","channels.slackAccessInstructions":"Map Slack channels to the team agents that should answer there.","channels.slackAccessAdd":"Add","channels.slackAccessLoading":"Loading Slack channels...","channels.slackAccessEmpty":"No Slack channels allowed yet.","channels.slackAccessAllow":"Remove {channelId}","channels.slackAccessAutoSubject":"Auto-generated team subject","channels.slackAccessNoSubjects":"No team agents available","channels.slackAccessSave":"Save channels","channels.slackAccessSaving":"Saving...","channels.slackAccessSuccess":"Slack channels saved.","channels.slackAccessError":"Slack channel update failed.","tools.permissions":"Tool permissions","tools.alwaysAllow":"Always allow","tools.askEachTime":"Ask each time","tools.disabled":"Disabled","tools.default":"default","tools.followDefault":"Follow global","tools.sourceDefault":"default permission","tools.sourceGlobal":"global setting","tools.sourceOverride":"per-tool override","tools.saved":"saved","tools.permissionFor":"Permission for {name}","tools.filterPlaceholder":"Filter tools\u2026","tools.noMatch":"No tools match the filter.","tools.failedLoad":"Failed to load tools: {message}","skills.installed":"Installed skills","skills.group.user":"Your skills","skills.group.system":"System skills","skills.group.workspace":"Workspace skills","skills.source.user":"user","skills.source.installed":"installed","skills.source.system":"system","skills.source.workspace":"workspace","skills.noInstalled":"No skills installed","skills.noInstalledDesc":"Skills extend the agent with domain-specific instructions. Add a SKILL.md bundle or place SKILL.md files in your workspace.","skills.failedLoad":"Failed to load skills: {message}","skills.import":"Add skill","skills.importDesc":"Paste SKILL.md content to add a user-mounted skill.","skills.name":"Skill name","skills.namePlaceholder":"skill-name","skills.url":"HTTPS URL","skills.urlHint":"Use a direct HTTPS link to a SKILL.md or supported skill bundle.","skills.urlPlaceholder":"https://example.com/SKILL.md","skills.httpsRequired":"URL must use HTTPS.","skills.importSourceRequired":"Provide an HTTPS URL or SKILL.md content.","skills.content":"SKILL.md content","skills.contentHint":"Use the full SKILL.md frontmatter and prompt content.","skills.contentPlaceholder":"---\\nname: example\\ndescription: ...\\n---\\n","skills.install":"Add","skills.installing":"Adding...","skills.installFailed":"Add failed.","skills.installedSuccess":'Added skill "{name}"',"skills.nameRequired":"Skill name is required.","skills.contentRequired":"SKILL.md content is required.","skills.remove":"Remove","skills.delete":"Delete","skills.edit":"Edit","skills.loading":"Loading...","skills.save":"Save","skills.saving":"Saving...","skills.cancel":"Cancel","skills.confirmRemove":'Remove skill "{name}"?',"skills.confirmDelete":'Delete skill "{name}"?',"skills.removeFailed":"Remove failed.","skills.removed":'Removed skill "{name}"',"skills.contentLoadFailed":"Failed to load SKILL.md content.","skills.updateFailed":"Update failed.","skills.updated":'Updated skill "{name}"',"skills.activatesOn":"Activates on","skills.imported":"imported","skills.defaultAutoActivationEnabled":"Default skill auto-activation enabled","skills.defaultAutoActivationDisabled":"Default skill auto-activation disabled","skills.defaultAutoActivationOnDesc":"Skills auto-activate by keyword on matching requests. Turn off to require an explicit /name.","skills.defaultAutoActivationOffDesc":"Skills run only when you type /name. Turn on to let them auto-activate by keyword.","skills.defaultAutoActivationOnButton":"Default: On","skills.defaultAutoActivationOffButton":"Default: Off","skills.autoActivateOnTitle":"Auto-activation on \u2014 runs on matching requests. Click to make it explicit-only (/name).","skills.autoActivateOffTitle":"Explicit-only \u2014 runs only when you type /name. Click to enable auto-activation.","skills.autoActivateOnLabel":"Auto-activate: On","skills.autoActivateOffLabel":"Auto-activate: Off","users.title":"Users ({count})","users.addUser":"Add user","users.newUser":"New user","users.displayName":"Display name","users.email":"Email","users.role":"Role","users.member":"Member","users.admin":"Admin","users.createUser":"Create user","users.creating":"Creating\u2026","users.cancel":"Cancel","users.adminRequired":"Admin access required","users.adminRequiredDesc":"User management is only available to accounts with admin privileges.","users.failedLoad":"Failed to load users: {message}","users.noUsers":"No users registered.","workspace.title":"Workspace","workspace.subtitle":"Memory, files & attachments","workspace.readOnly":"Read-only","workspace.filterPlaceholder":"Filter by name\u2026","workspace.emptyDir":"This folder is empty.","workspace.refresh":"Refresh","workspace.refreshing":"Refreshing","workspace.loading":"Loading...","workspace.noFiles":"No files here.","workspace.noMatches":"Nothing matches that filter.","workspace.breadcrumbRoot":"workspace","workspace.pickFileTitle":"Pick a file","workspace.pickFileDesc":"Choose a file from the tree to preview or download it. This viewer is read-only.","workspace.parent":"Parent: {path}","workspace.download":"Download","workspace.binaryPreviewUnavailable":"No inline preview for this file type. Download it to view the contents.","workspace.fileMeta":"{mime} \xB7 {size} bytes","workspace.unableOpenDirectory":"Unable to open directory","jobs.allJobs":"All jobs","jobs.refresh":"Refresh","jobs.refreshing":"Refreshing","jobs.unavailable":"Job unavailable","jobs.unavailableDesc":"This job no longer exists or is outside your access scope.","jobs.returnToJobs":"Return to jobs","jobs.dismiss":"Dismiss","jobs.list.explorer":"Explorer","jobs.list.queueTitle":"Job queue","jobs.list.queueDesc":"Search by title or ID, jump into a run, and stop active work without leaving the page.","jobs.list.visible":"{count} visible","jobs.list.state.live":"live","jobs.list.state.refreshing":"refreshing","jobs.list.searchPlaceholder":"Search job title or UUID","jobs.list.empty.noMatchTitle":"No jobs match the current filters","jobs.list.empty.noMatchDesc":"Try a broader search term or reset the state filter to see the rest of the queue.","jobs.list.empty.noJobsTitle":"No jobs yet","jobs.list.empty.noJobsDesc":"Background work, sandbox runs, and operator interventions will appear here once the gateway starts creating jobs.","jobs.list.filter.all":"All states","jobs.list.filter.pending":"Pending","jobs.list.filter.inProgress":"In progress","jobs.list.filter.completed":"Completed","jobs.list.filter.failed":"Failed","jobs.list.filter.stuck":"Stuck","jobs.list.untitled":"Untitled job","jobs.list.created":"created {value}","jobs.list.started":"started {value}","jobs.action.cancel":"Cancel","jobs.action.open":"Open","jobs.detail.backToAll":"Back to all jobs","jobs.detail.tabs.overview":"Overview","jobs.detail.tabs.activity":"Activity","jobs.detail.tabs.files":"Files","missions.allMissions":"All missions","missions.refresh":"Refresh","missions.refreshing":"Refreshing","missions.title":"Missions","missions.subtitle":"Execution loops","missions.summary":"{missions} missions across {projects} project workspaces.","missions.searchPlaceholder":"Search missions","missions.filter.status":"Status","missions.filter.project":"Project","missions.filter.allStatuses":"All statuses","missions.filter.allProjects":"All projects","missions.status.active":"Active","missions.status.paused":"Paused","missions.status.failed":"Failed","missions.status.completed":"Completed","missions.noGoal":"No mission goal set.","missions.threadCount":"{count} threads","missions.updated":"Updated {value}","missions.emptyTitle":"No missions match","missions.emptyDesc":"Adjust the search or filters to find a mission loop.","missions.unavailable":"Mission unavailable","missions.unavailableDesc":"This mission no longer exists or is outside your access scope.","missions.dossier":"Mission dossier","missions.meta.cadence":"Cadence","missions.meta.manual":"manual","missions.meta.threadsToday":"Threads today","missions.meta.unlimited":"unlimited","missions.meta.nextFire":"Next fire","missions.meta.updated":"Updated","missions.action.fireNow":"Fire now","missions.action.pause":"Pause","missions.action.resume":"Resume","missions.action.runOnce":"Run once","missions.action.runAgain":"Run again","missions.brief":"Brief","missions.currentFocus":"Current focus","missions.successCriteria":"Success criteria","missions.spawnedThreads":"Spawned threads","missions.summary.totalMissions":"Total missions","missions.summary.active":"Active","missions.summary.paused":"Paused","missions.summary.spawnedThreads":"Spawned threads","missions.summary.completedFailed":"{completed} completed / {failed} failed","missions.summary.acrossProjects":"Across every project workspace","automations.eyebrow":"Scheduled work","automations.title":"Automations","automations.description":"Scheduled automations only.","automations.filterLabel":"Automation status filter","automations.filter.all":"All","automations.filter.active":"Active","automations.filter.running":"Running","automations.filter.failures":"Failures","automations.filter.paused":"Paused","automations.filter.completed":"Completed","automations.refresh":"Refresh automations","automations.error.loadFailed":"Unable to load automations","automations.schedulerOff.title":"Scheduling is turned off","automations.schedulerOff.description":"These automations are saved but won't run until the scheduler is enabled.","automations.schedule.custom":"Custom schedule","automations.schedule.everyMinute":"Every minute","automations.schedule.everyMinutes":"Every {count} minutes","automations.schedule.hourlyAt":"Hourly at :{minute}","automations.schedule.everyDayAt":"Every day at {time}","automations.schedule.weekdaysAt":"Weekdays at {time}","automations.schedule.weekdayAt":"{weekday} at {time}","automations.schedule.monthlyAt":"Day {day} of each month at {time}","automations.schedule.dateAt":"{date} at {time}","automations.schedule.onceAt":"Once on {datetime}","automations.badge.muted":"Muted","automations.badge.signal":"Signal","automations.badge.info":"Info","automations.badge.danger":"Danger","automations.badge.success":"Success","automations.state.active":"Active","automations.state.scheduled":"Scheduled","automations.state.paused":"Paused","automations.state.disabled":"Disabled","automations.state.inactive":"Inactive","automations.state.completed":"Completed","automations.state.unknown":"Unknown","automations.lastStatus.done":"Done","automations.lastStatus.error":"Error","automations.lastStatus.running":"Running","automations.lastStatus.none":"No result","automations.runStatus.ok":"OK","automations.runStatus.error":"Error","automations.runStatus.running":"Running","automations.runStatus.unknown":"Unknown","automations.date.unknown":"Unknown","automations.date.notScheduled":"Not scheduled","automations.date.noRuns":"No runs yet","automations.date.unscheduled":"Unscheduled","automations.date.notSubmitted":"Not submitted","automations.date.notCompleted":"Not completed","automations.untitled":"Untitled automation","automations.successRate.none":"No completed runs","automations.successRate.visible":"{percent}% visible runs","automations.delivery.eyebrow":"Delivery defaults","automations.delivery.title":"Where triggered results are sent","automations.delivery.explainer":"Choose where automation results are delivered when a triggered run finishes.","automations.delivery.currentDefault":"Current default","automations.delivery.changeTarget":"Change target","automations.delivery.availableTargets":"Available targets","automations.delivery.none":"None","automations.delivery.webOption":"Web app only (no external delivery)","automations.delivery.webOptionDesc":"Results are stored in the run history. No DM or notification is sent.","automations.delivery.unpairedNotice":"Slack DM \u2014 not available","automations.delivery.unpairedDesc":"Pair your Slack account to enable DM delivery.","automations.delivery.save":"Save","automations.delivery.clear":"Clear","automations.delivery.saved":"Saved","automations.delivery.saveFailed":"Couldn't save the delivery target. Please try again.","automations.delivery.footnote":"Approval requests sent to your DM are answered by replying {command} in Slack.","automations.delivery.pill.ready":"Ready","automations.delivery.pill.unavailable":"Unavailable","automations.delivery.pill.notSet":"Not set","automations.delivery.pill.notPaired":"Not paired","automations.delivery.pill.fallback":"Fallback","automations.summary.scheduled":"Scheduled","automations.summary.scheduledDetail":"Scheduled automations visible to this agent.","automations.summary.active":"Active","automations.summary.activeDetail":"Enabled schedules waiting for their next run.","automations.summary.paused":"Paused","automations.summary.pausedDetail":"Schedules not currently expected to run.","automations.summary.running":"Running now","automations.summary.runningDetail":"Automations with a run in progress.","automations.summary.failures":"Failures","automations.summary.failuresDetail":"Automations with a failed run in recent history.","automations.summary.filterAction":"Show {label}","automations.summary.nextRun":"Next run","automations.summary.none":"None","automations.summary.nextRunDetail":"Soonest scheduled run in this list.","automations.empty.matchingTitle":"No matching automations","automations.empty.matchingDescription":"Try a different status filter.","automations.empty.noneTitle":"No scheduled automations yet.","automations.empty.noneDescription":"This agent has no scheduled work to show.","automations.empty.onboardingTitle":"No automations yet","automations.empty.onboardingDescription":"Automations are created by chatting with your agent \u2014 there's no form to fill out. Ask it to do something on a schedule and it will set up a recurring automation for you.","automations.empty.examplesTitle":"Try asking your agent","automations.empty.example1":"Check the nearai/ironclaw repo every 10 minutes and summarize new issues, PRs, and commits.","automations.empty.example2":"Every weekday at 9am, send me a summary of my unread email.","automations.empty.example3":"Remind me to review open pull requests every afternoon at 3pm.","automations.empty.startInChat":"Start in chat","automations.empty.copyPrompt":"Copy prompt","automations.empty.copied":"Copied","automations.refreshing":"Refreshing\u2026","automations.table.name":"Name","automations.table.schedule":"Schedule","automations.table.nextRun":"Next run","automations.table.lastRun":"Last run","automations.table.recentRuns":"Recent runs","automations.table.noRuns":"No runs","automations.table.status":"Status","automations.runs.total":"Recent runs: {count}","automations.runs.ok":"OK: {count}","automations.runs.error":"Failed: {count}","automations.runs.running":"Running: {count}","automations.runs.unknown":"Unknown: {count}","automations.runs.showingOf":"Showing {shown} of {total} recent runs","automations.status.running":"Running","automations.status.needsReview":"Needs review","automations.detail.emptyTitle":"Select an automation","automations.detail.emptyDescription":"Choose a schedule to inspect recent runs.","automations.detail.schedule":"Schedule","automations.detail.successRate":"Success rate","automations.detail.lastCompleted":"Last completed","automations.detail.currentRun":"Current run","automations.detail.noCurrentRun":"No active run","automations.detail.recentRuns":"Recent runs","automations.detail.noRuns":"This automation has not produced any visible runs yet.","automations.detail.openRun":"Open run","automations.detail.thread":"thread","automations.detail.run":"run","automations.detail.noThread":"No thread attached","routines.explorer":"Tasks","routines.title":"Routines","routines.description":"Search saved routines, inspect their schedule or trigger, and run or pause them without leaving v2.","ext.installed":"Installed","ext.channels":"Channels","ext.mcp":"MCP","ext.registry":"Registry","ext.registry.searchPlaceholder":"Search extensions\u2026","ext.registry.emptyTitle":"Registry is empty","ext.registry.emptyDesc":"All available extensions are already installed, or no registry is configured.","ext.registry.availableTitle":"Available extensions","ext.registry.noMatch":"No extensions match the filter.","chat.history.loading":"Loading...","chat.history.loadOlder":"Load older messages","projects.allProjects":"All projects","projects.returnToProjects":"Return to projects","projects.unavailable":"Project unavailable","projects.unavailableDesc":"This project no longer exists or is outside your access scope.","projects.refresh":"Refresh","projects.refreshing":"Refreshing","projects.newProject":"New project","projects.preparingChat":"Preparing chat...","projects.createFromChat":"Create from chat","projects.startProject":"Start a project","projects.searchPlaceholder":"Search projects","projects.creationDraft":"Create a new project for me. I want to set up a project for: ","projects.chatAutoFail":"Unable to prepare chat automatically. Opening chat anyway.","projects.openWorkspace":"Open project","projects.openGeneralWorkspace":"Open project","projects.noDescription":"No project description yet. The project is still being shaped by recent activity and thread history.","projects.general.label":"General project","projects.general.title":"Default project control room","projects.general.desc":"Shared context, ad hoc work, and the catch-all runtime path for threads that are not yet promoted into a named project.","projects.scoped.title":"Scoped projects","projects.scoped.desc":"Browse durable workspaces, inspect missions, review recent activity, and jump into the project that needs you now.","projects.scoped.onlyGeneralTitle":"Only the general workspace is active","projects.scoped.onlyGeneralDesc":"Create a named project when work deserves its own missions, files, widgets, and long-running context.","projects.empty.noMatchTitle":"No projects match the current search","projects.empty.noMatchDesc":"Try a broader search term or clear the filter to return to the full workspace map.","projects.empty.noneTitle":"No projects yet","projects.empty.noneDesc":"Projects appear once the assistant creates durable workspaces. You can start from chat and ask IronClaw to spin up a scoped project for ongoing work.","projects.card.runtime":"Runtime","projects.card.risk":"Risk","projects.card.threadsToday":"{count} today","projects.card.failures24h":"{count} in 24h","projects.card.spendToday":"{value} spend today","projects.explorer":"Explorer","lang.title":"Language","lang.description":"Choose the display language for the interface.","lang.current":"Current language","inference.provider":"LLM provider","inference.backend":"Backend","inference.model":"Model","inference.active":"active","inference.none":"\u2014","pairing.title":"Pairing","pairing.instructions":"Enter the code from the channel to finish pairing.","pairing.placeholder":"Enter pairing code\u2026","pairing.approve":"Approve","pairing.success":"Pairing complete.","pairing.error":"Pairing failed.","pairing.none":"No pending pairing requests.","pairing.slackTitle":"Slack account connection","pairing.slackInstructions":"Message the Slack app, then enter the code here.","pairing.slackPlaceholder":"Enter Slack pairing code\u2026","pairing.connect":"Connect","pairing.slackSuccess":"Slack account connected.","pairing.slackError":"Invalid or expired Slack pairing code.","admin.tab.dashboard":"Dashboard","admin.tab.users":"Users","admin.tab.usage":"Usage","admin.dashboard.systemOverview":"System overview","admin.dashboard.uptime":"Uptime: {value}","admin.dashboard.totalUsers":"Total users","admin.dashboard.activeUsers":"Active users","admin.dashboard.suspended":"Suspended","admin.dashboard.admins":"Admins","admin.dashboard.usage30d":"30-day usage","admin.dashboard.totalJobs":"Total jobs","admin.dashboard.activeJobs":"Active jobs","admin.dashboard.llmCalls":"LLM calls","admin.dashboard.totalCost":"Total cost","admin.dashboard.recentUsers":"Recent users","admin.dashboard.viewAll":"View all","admin.dashboard.noUsers":"No users yet.","admin.dashboard.name":"Name","admin.dashboard.role":"Role","admin.dashboard.status":"Status","admin.dashboard.jobs":"Jobs","admin.dashboard.lastActive":"Last active","admin.users.user":"user","admin.users.userFallback":"user","admin.users.title":"Users ({count} / {total})","admin.users.searchPlaceholder":"Search\u2026","admin.users.noMatch":"No users match the current filters.","admin.users.filter.all":"All","admin.users.filter.active":"Active","admin.users.filter.suspended":"Suspended","admin.users.filter.admins":"Admins","admin.users.newUser":"New user","admin.users.createUser":"Create user","admin.users.creating":"Creating\u2026","admin.users.cancel":"Cancel","admin.users.displayName":"Display name","admin.users.displayNamePlaceholder":"Jane Doe","admin.users.email":"Email","admin.users.emailPlaceholder":"jane@example.com","admin.users.role":"Role","admin.users.member":"Member","admin.users.admin":"Admin","admin.users.suspend":"Suspend","admin.users.activate":"Activate","admin.users.promote":"Promote","admin.users.demote":"Demote","admin.users.token":"Token","admin.users.jobsCount":"{count} jobs","admin.users.suspendTitle":"Suspend user","admin.users.suspendDesc":"This will prevent the user from authenticating. Continue?","admin.users.tokenNamePrompt":"Token name for {name}:","admin.users.tokenCreated":"Token created","admin.users.tokenCreatedDesc":"Copy this now \u2014 it will not be shown again.","admin.users.copy":"Copy","admin.users.copied":"Copied","admin.users.backToUsers":"Back to users","admin.users.createToken":"Create token","admin.users.delete":"Delete","admin.users.deleteUserTitle":"Delete user","admin.users.deleteUserDesc":'Are you sure you want to delete "{name}"? This action cannot be undone.',"admin.user.profile":"Profile","admin.user.summary":"Summary","admin.user.id":"ID","admin.user.email":"Email","admin.user.created":"Created","admin.user.lastLogin":"Last login","admin.user.createdBy":"Created by","admin.user.notSet":"Not set","admin.user.jobs":"Jobs","admin.user.totalCost":"Total cost","admin.user.lastActive":"Last active","admin.user.roleManagement":"Role management","admin.user.currentRole":"Current role","admin.user.saveRole":"Save role","admin.user.usage30Days":"Usage (last 30 days)","admin.user.noUsage":"No usage data.","admin.usage.overview":"Usage overview","admin.usage.noData":"No usage data for this period.","admin.usage.totalCalls":"Total calls","admin.usage.inputTokens":"Input tokens","admin.usage.outputTokens":"Output tokens","admin.usage.totalCost":"Total cost","admin.usage.perUser":"Per-user breakdown","admin.usage.perModel":"Per-model breakdown","admin.usage.user":"User","admin.usage.model":"Model","admin.usage.calls":"Calls","admin.usage.input":"Input","admin.usage.output":"Output","admin.usage.cost":"Cost","logs.levelAll":"All levels","logs.level.trace":"TRACE","logs.level.debug":"DEBUG","logs.level.info":"INFO","logs.level.warn":"WARN","logs.level.error":"ERROR","logs.filterTarget":"Filter by target\u2026","logs.autoScroll":"Auto-scroll","logs.pause":"Pause","logs.resume":"Resume","logs.clear":"Clear","logs.confirmClear":"Clear all log entries?","logs.scoped":"Scoped logs","logs.scope.thread":"Thread","logs.scope.run":"Run","logs.scope.turn":"Turn","logs.scope.toolCall":"Tool call","logs.scope.tool":"Tool","logs.scope.source":"Source","logs.clearScope":"Clear scope","logs.serverLevel":"Server level:","logs.entryCount":"{count} entries","logs.pausedBadge":"\u25CF paused","logs.empty":"Waiting for log entries\u2026","common.recent":"Recent","common.searchChats":"Search chats...","common.gatewaySession":"Gateway session","common.pinned":"Pinned","common.deleteChat":"Delete chat","chat.deleteFailed":"Couldn't delete this conversation.","chat.deleteBusy":"Can't delete a conversation while it's still running. Stop it first, then try again.","command.placeholder":"Type a command or search...","routine.searchPlaceholder":"Search routine name, trigger, or action","routine.unavailable":"Routine unavailable","routine.unavailableDesc":"This routine no longer exists or is outside your access scope.","routine.triggerPayload":"Trigger payload","routine.actionPayload":"Action payload","job.noWorkspace":"No project workspace","job.noFile":"No file selected","job.noActivityTitle":"No activity captured yet","job.noActivityDesc":"This job has not written any persisted events for the selected filter.","job.noStateTitle":"No state history yet","job.followupPlaceholder":"Send a follow-up prompt to the running job","common.noChatsMatch":'No chats match "{query}"',"extensions.configure":"Configure","extensions.reconfigure":"Reconfigure","extensions.configureName":"Configure {name}","extensions.allInstalled":"All installed extensions","mcp.installed":"Installed MCP servers","extensions.oneCapability":"1 capability","extensions.pluralCapabilities":"{count} capabilities","extensions.oneKeyword":"1 keyword","extensions.pluralKeywords":"{count} keywords","extensions.moreActions":"More actions","extensions.kind.wasm_tool":"WASM Tool","extensions.kind.wasm_channel":"Channel","extensions.kind.channel":"Channel","extensions.kind.mcp_server":"MCP Server","extensions.kind.first_party":"First-party","extensions.kind.system":"System","extensions.kind.channel_relay":"Relay","extensions.state.active":"active","extensions.state.ready":"ready","extensions.state.pairing_required":"pairing","extensions.state.pairing":"pairing","extensions.state.auth_required":"auth needed","extensions.state.setup_required":"setup needed","extensions.state.failed":"failed","extensions.state.installed":"installed","extensions.state.available":"available","extensions.loadFailed":"Failed to load setup:","extensions.noConfigRequired":"No configuration required for this extension.","common.optional":"optional","common.configured":"configured","extensions.autoGenerated":"Auto-generated if left blank","extensions.activeConfigured":"Extension is active.","extensions.authConfigured":"Authorization is configured.","extensions.authPopup":"Authorize this provider in a browser popup.","extensions.opening":"Opening...","extensions.authorize":"Authorize","extensions.reauthorize":"Reauthorize","extensions.reconnect":"Reconnect","extensions.emptyInstalledTitle":"No extensions installed","extensions.emptyInstalledDesc":"Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.","extensions.emptyMcpTitle":"No MCP servers","extensions.emptyMcpDesc":"MCP servers extend the agent with additional tool capabilities over the Model Context Protocol. Install them from the registry.","common.dismiss":"Dismiss","common.pin":"Pin","common.unpin":"Unpin","common.remove":"Remove"});(0,lR.createRoot)(document.getElementById("v2-root")).render(l`
  <${Vh}>
    <${Td} client=${At}>
      <${oR} />
    <//>
  <//>
`);
