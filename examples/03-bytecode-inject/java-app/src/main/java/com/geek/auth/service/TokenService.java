package com.geek.auth.service;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.nio.charset.StandardCharsets;
import java.util.Base64;

public class TokenService {

    private final String secret;
    private final long expireMs;

    public TokenService(String secret, int expireHours) {
        this.secret = secret;
        this.expireMs = expireHours * 3600_000L;
    }

    public String createToken(String username) {
        long exp = System.currentTimeMillis() + expireMs;
        String payload = username + ":" + exp;
        String sig = hmacSha256(payload);
        return Base64.getUrlEncoder().withoutPadding().encodeToString(
                (payload + ":" + sig).getBytes(StandardCharsets.UTF_8));
    }

    public boolean validateToken(String token) {
        try {
            String decoded = new String(Base64.getUrlDecoder().decode(token), StandardCharsets.UTF_8);
            String[] parts = decoded.split(":", 3);
            if (parts.length != 3) return false;
            long exp = Long.parseLong(parts[1]);
            if (System.currentTimeMillis() > exp) return false;
            String expectedSig = hmacSha256(parts[0] + ":" + parts[1]);
            return expectedSig.equals(parts[2]);
        } catch (Exception e) {
            return false;
        }
    }

    public String getUsernameFromToken(String token) {
        try {
            String decoded = new String(Base64.getUrlDecoder().decode(token), StandardCharsets.UTF_8);
            return decoded.split(":", 3)[0];
        } catch (Exception e) {
            return null;
        }
    }

    private String hmacSha256(String data) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(secret.getBytes(StandardCharsets.UTF_8), "HmacSHA256"));
            byte[] hash = mac.doFinal(data.getBytes(StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder();
            for (byte b : hash) sb.append(String.format("%02x", b));
            return sb.toString();
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }
}
