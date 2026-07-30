#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/uio.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 2 && argc != 3) {
        return 2;
    }

    const char *mode = argc == 3 ? argv[2] : "tcp";
    int use_udp =
        strcmp(mode, "udp") == 0
        || strcmp(mode, "udp-connected") == 0
        || strcmp(mode, "udp-connect-only") == 0;
#if defined(__linux__)
    use_udp = use_udp || strcmp(mode, "udp-sendmmsg") == 0;
#endif
    use_udp =
        use_udp
        || strcmp(mode, "udp-write") == 0
        || strcmp(mode, "udp-writev") == 0;
    int fd = socket(AF_INET, use_udp ? SOCK_DGRAM : SOCK_STREAM, 0);
    if (fd < 0) {
        return 3;
    }
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0) {
        close(fd);
        return 5;
    }
    struct sockaddr_in address;
    memset(&address, 0, sizeof(address));
    address.sin_family = AF_INET;
    address.sin_port = htons(9);
    if (inet_pton(AF_INET, argv[1], &address.sin_addr) != 1) {
        close(fd);
        return 4;
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
            sizeof(address)
        );
    } else {
        network_status = connect(fd, (const struct sockaddr *)&address, sizeof(address));
        if (network_status == 0 && strcmp(mode, "udp-connected") == 0) {
            const char byte = 'x';
            network_status = (int)send(fd, &byte, sizeof(byte), 0);
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
