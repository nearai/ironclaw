#define _GNU_SOURCE

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/uio.h>
#include <unistd.h>

#if defined(__linux__)
#include <sys/sendfile.h>
#endif

int main(int argc, char **argv) {
    if (argc != 2 && argc != 3) {
        return 2;
    }

    const char *mode = argc == 3 ? argv[2] : "tcp";
    int use_udp =
        strcmp(mode, "udp") == 0
        || strcmp(mode, "udp-connected") == 0
        || strcmp(mode, "udp-connect-only") == 0
        || strcmp(mode, "udp-sendfile") == 0;
#if defined(__linux__)
    use_udp = use_udp || strcmp(mode, "udp-sendmmsg") == 0;
#endif
    use_udp =
        use_udp
        || strcmp(mode, "udp-write") == 0
        || strcmp(mode, "udp-writev") == 0;
    int family = strchr(argv[1], ':') == NULL ? AF_INET : AF_INET6;
    int fd = socket(family, use_udp ? SOCK_DGRAM : SOCK_STREAM, 0);
    if (fd < 0) {
        return 3;
    }
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0) {
        close(fd);
        return 5;
    }
    struct sockaddr_storage address;
    memset(&address, 0, sizeof(address));
    socklen_t address_length;
    if (family == AF_INET) {
        struct sockaddr_in *ipv4 = (struct sockaddr_in *)&address;
        ipv4->sin_family = AF_INET;
        ipv4->sin_port = htons(9);
        address_length = sizeof(*ipv4);
        if (inet_pton(AF_INET, argv[1], &ipv4->sin_addr) != 1) {
            close(fd);
            return 4;
        }
    } else {
        struct sockaddr_in6 *ipv6 = (struct sockaddr_in6 *)&address;
        ipv6->sin6_family = AF_INET6;
        ipv6->sin6_port = htons(9);
        address_length = sizeof(*ipv6);
        if (inet_pton(AF_INET6, argv[1], &ipv6->sin6_addr) != 1) {
            close(fd);
            return 4;
        }
    }
    int network_status;
    if (strcmp(mode, "udp") == 0) {
        const char byte = 'x';
        network_status = (int)sendto(
            fd,
            &byte,
            sizeof(byte),
            0,
            (const struct sockaddr *)&address,
            address_length
        );
    } else {
        network_status =
            connect(fd, (const struct sockaddr *)&address, address_length);
        if (network_status == 0 && strcmp(mode, "udp-connected") == 0) {
            const char byte = 'x';
            network_status = (int)send(fd, &byte, sizeof(byte), 0);
        } else if (network_status == 0 && strcmp(mode, "udp-sendfile") == 0) {
            const char *temporary_root = getenv("TMPDIR");
            if (temporary_root == NULL || temporary_root[0] == '\0') {
                close(fd);
                return 6;
            }
            char input_path[PATH_MAX];
            int path_length = snprintf(
                input_path,
                sizeof(input_path),
                "%s/ironclaw-sendfile-probe.XXXXXX",
                temporary_root
            );
            if (path_length < 0 || (size_t)path_length >= sizeof(input_path)) {
                close(fd);
                return 6;
            }
            int input_fd = mkstemp(input_path);
            if (input_fd < 0) {
                close(fd);
                return 7;
            }
            const char byte = 'x';
            if (write(input_fd, &byte, sizeof(byte)) != (ssize_t)sizeof(byte)) {
                close(input_fd);
                unlink(input_path);
                close(fd);
                return 8;
            }
#if defined(__APPLE__)
            off_t length = sizeof(byte);
            network_status = sendfile(input_fd, fd, 0, &length, NULL, 0);
#else
            off_t offset = 0;
            network_status = (int)sendfile(fd, input_fd, &offset, sizeof(byte));
#endif
            close(input_fd);
            unlink(input_path);
        } else if (network_status == 0 && strcmp(mode, "udp-write") == 0) {
            const char byte = 'x';
            network_status = (int)write(fd, &byte, sizeof(byte));
        } else if (network_status == 0 && strcmp(mode, "udp-writev") == 0) {
            char byte = 'x';
            const struct iovec iovec = {
                .iov_base = &byte,
                .iov_len = sizeof(byte),
            };
            network_status = (int)writev(fd, &iovec, 1);
#if defined(__linux__)
        } else if (network_status == 0 && strcmp(mode, "udp-sendmmsg") == 0) {
            char byte = 'x';
            struct iovec iovec = {
                .iov_base = &byte,
                .iov_len = sizeof(byte),
            };
            struct mmsghdr message;
            memset(&message, 0, sizeof(message));
            message.msg_hdr.msg_iov = &iovec;
            message.msg_hdr.msg_iovlen = 1;
            network_status = sendmmsg(fd, &message, 1, 0);
#endif
        }
    }
    if (network_status < 0 && errno == EPERM) {
        fprintf(
            stderr,
            "non-loopback network attempt rejected by hermetic process boundary\n"
        );
        close(fd);
        return 90;
    }
    close(fd);
    return 0;
}
