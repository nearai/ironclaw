#define _GNU_SOURCE

#include <arpa/inet.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

#if !defined(__APPLE__)
typedef int (*connect_fn)(int, const struct sockaddr *, socklen_t);
typedef ssize_t (*sendmsg_fn)(int, const struct msghdr *, int);
typedef ssize_t (*sendto_fn)(int, const void *, size_t, int, const struct sockaddr *, socklen_t);
static connect_fn real_connect(void) {
    static connect_fn function;
    if (function == NULL) {
        function = (connect_fn)dlsym(RTLD_NEXT, "connect");
    }
    return function;
}

static sendmsg_fn real_sendmsg(void) {
    static sendmsg_fn function;
    if (function == NULL) {
        function = (sendmsg_fn)dlsym(RTLD_NEXT, "sendmsg");
    }
    return function;
}

static sendto_fn real_sendto(void) {
    static sendto_fn function;
    if (function == NULL) {
        function = (sendto_fn)dlsym(RTLD_NEXT, "sendto");
    }
    return function;
}
#endif

static int is_loopback(const struct sockaddr *address) {
    if (address == NULL || address->sa_family == AF_UNSPEC) {
        return 1;
    }
    if (address->sa_family == AF_INET) {
        const struct sockaddr_in *ipv4 = (const struct sockaddr_in *)address;
        return (ntohl(ipv4->sin_addr.s_addr) & 0xff000000U) == 0x7f000000U;
    }
    if (address->sa_family == AF_INET6) {
        const struct sockaddr_in6 *ipv6 = (const struct sockaddr_in6 *)address;
        return IN6_IS_ADDR_LOOPBACK(&ipv6->sin6_addr);
    }
    /* Unix sockets and non-IP kernel transports are local by definition. */
    return 1;
}

static void record_violation(const struct sockaddr *address) {
    const char *path = getenv("IRONCLAW_HERMETIC_NETWORK_VIOLATIONS");
    if (path == NULL || path[0] == '\0') {
        return;
    }

    char ip[INET6_ADDRSTRLEN] = "unknown";
    if (address != NULL && address->sa_family == AF_INET) {
        const struct sockaddr_in *ipv4 = (const struct sockaddr_in *)address;
        (void)inet_ntop(AF_INET, &ipv4->sin_addr, ip, sizeof(ip));
    } else if (address != NULL && address->sa_family == AF_INET6) {
        const struct sockaddr_in6 *ipv6 = (const struct sockaddr_in6 *)address;
        (void)inet_ntop(AF_INET6, &ipv6->sin6_addr, ip, sizeof(ip));
    }

    char message[256];
    int length = snprintf(
        message,
        sizeof(message),
        "non-loopback network attempt: pid=%ld family=%d address=%s\n",
        (long)getpid(),
        address == NULL ? -1 : address->sa_family,
        ip
    );
    if (length <= 0) {
        return;
    }
    if ((size_t)length >= sizeof(message)) {
        length = (int)sizeof(message) - 1;
    }

    int fd = open(path, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (fd >= 0) {
        (void)write(fd, message, (size_t)length);
        (void)close(fd);
    }
}

static int guarded_connect(
    int socket_fd,
    const struct sockaddr *address,
    socklen_t address_length
) {
    if (!is_loopback(address)) {
        record_violation(address);
        errno = EPERM;
        return -1;
    }
#if defined(__APPLE__)
    return (int)syscall(SYS_connect, socket_fd, address, address_length);
#else
    connect_fn function = real_connect();
    if (function == NULL) {
        errno = ENOSYS;
        return -1;
    }
    return function(socket_fd, address, address_length);
#endif
}

static ssize_t guarded_sendmsg(int socket_fd, const struct msghdr *message, int flags) {
    const struct sockaddr *address =
        message == NULL ? NULL : (const struct sockaddr *)message->msg_name;
    if (!is_loopback(address)) {
        record_violation(address);
        errno = EPERM;
        return -1;
    }
#if defined(__APPLE__)
    return (ssize_t)syscall(SYS_sendmsg, socket_fd, message, flags);
#else
    sendmsg_fn function = real_sendmsg();
    if (function == NULL) {
        errno = ENOSYS;
        return -1;
    }
    return function(socket_fd, message, flags);
#endif
}

static ssize_t guarded_sendto(
    int socket_fd,
    const void *buffer,
    size_t length,
    int flags,
    const struct sockaddr *address,
    socklen_t address_length
) {
    if (!is_loopback(address)) {
        record_violation(address);
        errno = EPERM;
        return -1;
    }
#if defined(__APPLE__)
    return (ssize_t)syscall(
        SYS_sendto,
        socket_fd,
        buffer,
        length,
        flags,
        address,
        address_length
    );
#else
    sendto_fn function = real_sendto();
    if (function == NULL) {
        errno = ENOSYS;
        return -1;
    }
    return function(socket_fd, buffer, length, flags, address, address_length);
#endif
}

#if defined(__APPLE__)
#define DYLD_INTERPOSE(replacement, replacee)                                      \
    __attribute__((used)) static struct {                                          \
        const void *replacement;                                                   \
        const void *replacee;                                                      \
    } _interpose_##replacee __attribute__((section("__DATA,__interpose"))) = {     \
        (const void *)(unsigned long)&replacement,                                 \
        (const void *)(unsigned long)&replacee                                     \
    }

DYLD_INTERPOSE(guarded_connect, connect);
DYLD_INTERPOSE(guarded_sendmsg, sendmsg);
DYLD_INTERPOSE(guarded_sendto, sendto);
#else
int connect(int socket_fd, const struct sockaddr *address, socklen_t address_length) {
    return guarded_connect(socket_fd, address, address_length);
}

ssize_t sendmsg(int socket_fd, const struct msghdr *message, int flags) {
    return guarded_sendmsg(socket_fd, message, flags);
}

ssize_t sendto(
    int socket_fd,
    const void *buffer,
    size_t length,
    int flags,
    const struct sockaddr *address,
    socklen_t address_length
) {
    return guarded_sendto(socket_fd, buffer, length, flags, address, address_length);
}
#endif
