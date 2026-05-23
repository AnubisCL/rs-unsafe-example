package com.geek.auth.controller;

import com.geek.auth.model.LoginResult;
import com.geek.auth.service.AuthService;
import org.springframework.stereotype.Controller;
import org.springframework.web.bind.annotation.*;

@Controller
public class LoginController {

    private final AuthService authService;

    public LoginController(AuthService authService) {
        this.authService = authService;
    }

    @GetMapping("/")
    public String loginPage() {
        return "login";
    }

    @ResponseBody
    @PostMapping("/api/login")
    public LoginResult login(@RequestParam String username, @RequestParam String password) {
        return authService.login(username, password);
    }

    @ResponseBody
    @GetMapping("/api/test")
    public String test(@RequestAttribute("username") String username) {
        return "Hello " + username + "! You have accessed a protected resource.";
    }
}
