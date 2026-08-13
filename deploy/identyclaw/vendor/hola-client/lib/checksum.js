/** HOLA line checksum: sum(char codes of UTF-16 units) mod 23 → one letter (omits I, L, O). */
const HOLA_CHECKSUM_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ";

function computeHolaChecksum(prefix) {
  let sum = 0;
  for (let i = 0; i < prefix.length; i += 1) {
    sum += prefix.charCodeAt(i);
  }
  return HOLA_CHECKSUM_ALPHABET[sum % HOLA_CHECKSUM_ALPHABET.length];
}

module.exports = {
  HOLA_CHECKSUM_ALPHABET,
  computeHolaChecksum
};
