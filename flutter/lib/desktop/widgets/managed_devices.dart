import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:http/http.dart' as http;

class ManagedDevices extends StatefulWidget {
  const ManagedDevices({super.key, required this.api});

  final String api;

  @override
  State<ManagedDevices> createState() => _ManagedDevicesState();
}

class _ManagedDevicesState extends State<ManagedDevices> {
  final _username = TextEditingController();
  final _password = TextEditingController();
  final _http = http.Client();
  String? _token;
  List<Map<String, dynamic>> _devices = [];
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _http.close();
    _username.dispose();
    _password.dispose();
    super.dispose();
  }

  Future<Map<String, dynamic>> _request(String method, String path,
      {Map<String, String>? body}) async {
    final url =
        Uri.parse('${widget.api.replaceFirst(RegExp(r'/+$'), '')}$path');
    final request = http.Request(method, url)..followRedirects = false;
    final token = _token;
    if (token != null) request.headers['Authorization'] = 'Bearer $token';
    request.headers['Accept'] = 'application/json';
    if (body != null) {
      request.headers['Content-Type'] = 'application/json';
      request.body = jsonEncode(body);
    }
    const timeout = Duration(seconds: 10);
    final response = await _http.send(request).timeout(timeout);
    final result = await http.Response.fromStream(response).timeout(timeout);
    if (result.statusCode == 401) throw const _SignInRequired();
    if (result.statusCode != 200) throw const FormatException();
    return jsonDecode(utf8.decode(result.bodyBytes)) as Map<String, dynamic>;
  }

  Future<void> _run(Future<void> Function() action) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await action();
    } on _SignInRequired {
      if (!mounted) return;
      _token = null;
      _devices = [];
      _error = 'Sign in failed or expired.';
    } catch (_) {
      if (mounted)
        _error = 'The management API could not complete the request.';
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _signIn() async {
    final body = {
      'username': _username.text.trim(),
      'password': _password.text
    };
    _password.clear();
    final response = await _request('POST', '/v1/auth/login', body: body);
    if (!mounted) return;
    final token = response['access_token'] as String;
    if (token.isEmpty) throw const FormatException();
    _token = token;
    await _refresh();
  }

  Future<void> _refresh() async {
    final response = await _request('GET', '/v1/me/devices');
    if (!mounted) return;
    _devices = (response['devices'] as List).cast<Map<String, dynamic>>();
  }

  Future<void> _connect(String deviceId) async {
    final uuid =
        RegExp(r'^[0-9a-fA-F]{8}-(?:[0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}$');
    if (!uuid.hasMatch(deviceId)) {
      throw const FormatException();
    }
    final response = await _request('POST', '/v1/devices/$deviceId/connect');
    final id = response['rustdesk_id'] as String;
    final password = response['password'] as String;
    if (!RegExp(r'^[A-Za-z0-9_-]{1,64}$').hasMatch(id) || password.isEmpty) {
      throw const FormatException();
    }
    if (!mounted) return;
    await connect(context, id, password: password, isSharedPassword: true);
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
        title: const Text('Managed devices'),
        content: SizedBox(
          width: 420,
          height: _token == null ? null : 290,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (_token == null) ...[
                TextField(
                  controller: _username,
                  enabled: !_busy,
                  decoration: const InputDecoration(labelText: 'Username'),
                ),
                TextField(
                  controller: _password,
                  enabled: !_busy,
                  obscureText: true,
                  decoration: const InputDecoration(labelText: 'Password'),
                  onSubmitted: (_) => _run(_signIn),
                ),
              ] else if (_devices.isEmpty)
                const Text('No assigned devices.')
              else
                Flexible(
                  child: ListView(
                    shrinkWrap: true,
                    children: [
                      for (final device in _devices)
                        ListTile(
                          title:
                              Text('${device['name'] ?? device['device_id']}'),
                          subtitle: device['status'] == 'ready'
                              ? null
                              : Text('${device['status'] ?? 'unavailable'}'),
                          enabled: !_busy && device['status'] == 'ready',
                          onTap: () => _run(
                              () => _connect(device['device_id'] as String)),
                        ),
                    ],
                  ),
                ),
              if (_busy) const LinearProgressIndicator(),
              if (_error != null) Text(_error!),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('Close'),
          ),
          if (_token == null)
            FilledButton(
                onPressed: _busy ? null : () => _run(_signIn),
                child: const Text('Sign in'))
          else
            TextButton(
                onPressed: _busy ? null : () => _run(_refresh),
                child: const Text('Refresh')),
        ],
      );
}

class _SignInRequired implements Exception {
  const _SignInRequired();
}
