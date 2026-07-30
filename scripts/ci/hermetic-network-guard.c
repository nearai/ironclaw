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
typedef ssize_t (*send_fn)(int, const void *, size_t, int);
typedef ssize_t (*sendmsg_fn)(int, const struct msghdr *, int);
typedef ssize_t (*sendto_fn)(int, const void *, size_t, int, const struct sockaddr *, socklen_t);
static connect_fn real_connect(void) {
    return (connect_fn)dlsym(RTLD_NEXT, "connect");
}

static send_fn real_send(void) {
    return (send_fn)dlsym(RTLD_NEXT, "send");
}

static sendmsg_fn real_sendmsg(void) {
    return (sendmsg_fn)dlsym(RTLD_NEXT, "sendmsg");
}

static sendto_fn real_sendto(void) {
    return (sendto_fn)dlsym(RTLD_NEXT, "sendto");
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

static int socket_is_datagram(int socket_fd) {
    int socket_type = 0;
    socklen_t length = sizeof(socket_type);
    int saved_errno = errno;
    int status = getsockopt(socket_fd, SOL_SOCKET, SO_TYPE, &socket_type, &length);
    errno = saved_errno;
    return status == 0 && socket_type == SOCK_DGRAM;
}

static const struct sockaddr *non_loopback_destination(
    int socket_fd,
    const struct sockaddr *address,
    struct sockaddr_storage *peer
) {
    if (address != NULL) {
        return is_loopback(address) ? NULL : address;
    }

    socklen_t length = sizeof(*peer);
    int saved_errno = errno;
    int status = getpeername(socket_fd, (struct sockaddr *)peer, &length);
    errno = saved_errno;
    if (status == 0 && !is_loopback((const struct sockaddr *)peer)) {
        return (const struct sockaddr *)peer;
    }
    return NULL;
}

static void write_violation(int fd, const char *message, size_t length) {
    size_t offset = 0;
    while (offset < length) {
        ssize_t written = write(fd, message + offset, length - offset);
        if (written > 0) {
            offset += (size_t)written;
            continue;
        }
        if (written < 0 && errno == EINTR) {
            continue;
        }
        break;
    }
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
        write_violation(fd, message, (size_t)length);
        int close_status = close(fd);
        (void)close_status;
    }
}

static int guarded_connect(
    int socket_fd,
    const struct sockaddr *address,
    socklen_t address_length
) {
    /*
     * UDP connect() sends no packet; Chromium uses it to inspect route/source
     * selection. Actual connected UDP writes are rejected by the send guards.
     */
    if (!is_loopback(address) && !socket_is_datagram(socket_fd)) {
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

static ssize_t guarded_send(
    int socket_fd,
    const void *buffer,
    size_t length,
    int flags
) {
    struct sockaddr_storage peer;
    const struct sockaddr *blocked =
        non_loopback_destination(socket_fd, NULL, &peer);
    if (blocked != NULL) {
        record_violation(blocked);
        errno = EPERM;
        return -1;
    }
#if defined(__APPLE__)
    return (ssize_t)syscall(SYS_sendto, socket_fd, buffer, length, flags, NULL, 0);
#else
    send_fn function = real_send();
    if (function == NULL) {
        errno = ENOSYS;
        return -1;
    }
    return function(socket_fd, buffer, length, flags);
#endif
}

static ssize_t guarded_sendmsg(int socket_fd, const struct msghdr *message, int flags) {
    const struct sockaddr *address =
        message == NULL ? NULL : (const struct sockaddr *)message->msg_name;
    struct sockaddr_storage peer;
    const struct sockaddr *blocked =
        non_loopback_destination(socket_fd, address, &peer);
    if (blocked != NULL) {
        record_violation(blocked);
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
    struct sockaddr_storage peer;
    const struct sockaddr *blocked =
        non_loopback_destination(socket_fd, address, &peer);
    if (blocked != NULL) {
        record_violation(blocked);
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
DYLD_INTERPOSE(guarded_send, send);
DYLD_INTERPOSE(guarded_sendmsg, sendmsg);
DYLD_INTERPOSE(guarded_sendto, sendto);
#else
int connect(int socket_fd, const struct sockaddr *address, socklen_t address_length) {
    return guarded_connect(socket_fd, address, address_length);
}

ssize_t send(int socket_fd, const void *buffer, size_t length, int flags) {
    return guarded_send(socket_fd, buffer, length, flags);
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
