#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 2 && argc != 3) {
        return 2;
    }

    int use_udp = argc == 3 && strcmp(argv[2], "udp") == 0;
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
    if (use_udp) {
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
