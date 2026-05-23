package com.geek.auth.service;

import com.geek.auth.model.LoginResult;
import com.geek.auth.model.User;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;
import org.yaml.snakeyaml.Yaml;

import jakarta.annotation.PostConstruct;
import java.io.InputStream;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.List;
import java.util.Map;

@Service
public class AuthService {

    @Value("${auth.token.secret}")
    private String secret;

    @Value("${auth.token.expire-hours}")
    private int expireHours;

    private List<User> users;
    private TokenService tokenService;

    @PostConstruct
    public void init() {
        tokenService = new TokenService(secret, expireHours);
        Yaml yaml = new Yaml();
        try (InputStream is = getClass().getClassLoader().getResourceAsStream("users.yml")) {
            Map<String, Object> data = yaml.load(is);
            @SuppressWarnings("unchecked")
            List<Map<String, String>> userList = (List<Map<String, String>>) data.get("users");
            users = userList.stream().map(m -> {
                User u = new User();
                u.setUsername(m.get("username"));
                u.setPasswordMd5(m.get("password-md5"));
                return u;
            }).toList();
        } catch (Exception e) {
            throw new RuntimeException("Failed to load users.yml", e);
        }
    }

    public LoginResult login(String username, String password) {
        String md5 = md5(password);
        for (User user : users) {
            if (user.getUsername().equals(username) && user.getPasswordMd5().equals(md5)) {
                String token = tokenService.createToken(username);
                return LoginResult.ok(token);
            }
        }
        return LoginResult.fail("invalid username or password");
    }

    public boolean validateToken(String token) {
        return tokenService.validateToken(token);
    }

    public String getUsernameFromToken(String token) {
        return tokenService.getUsernameFromToken(token);
    }

    private String md5(String input) {
        try {
            MessageDigest md = MessageDigest.getInstance("MD5");
            byte[] digest = md.digest(input.getBytes(StandardCharsets.UTF_8));
            return String.format("%032x", new BigInteger(1, digest));
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }
}
